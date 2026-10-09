//! A single ordered storage worker. State transitions enqueue while holding
//! their short state lock; disk/keychain work never runs under that lock.
use super::{records::SessionRepository, Service};
use crate::{auth::AuthSession, client::CookieClientParts, keychain::StoreError};
use std::sync::{mpsc, Arc};

type Loaded = Result<Option<(CookieClientParts, Option<AuthSession>)>, StoreError>;
enum Job {
    Load(Service, mpsc::Sender<Loaded>),
    Save(
        Service,
        Arc<reqwest_cookie_store::CookieStoreMutex>,
        Option<AuthSession>,
    ),
    Delete(Service),
    Barrier(mpsc::Sender<Result<(), String>>),
}

pub(super) struct Writer(mpsc::Sender<Job>);
impl Writer {
    pub fn new(repository: Box<dyn SessionRepository>) -> Self {
        let (send, receive) = mpsc::channel();
        std::thread::Builder::new()
            .name("session-storage".into())
            .spawn(move || {
                let mut failures: [Option<String>; 3] = Default::default();
                let mut failed_deletes = [false; 3];
                while let Ok(job) = receive.recv() {
                    let (service, result) = match job {
                        Job::Load(service, reply) => {
                            let _ = reply.send(repository.load(service));
                            continue;
                        }
                        Job::Save(service, cookies, identity) => {
                            failed_deletes[service.index()] = false;
                            (
                                service,
                                repository.save(service, &cookies, identity.as_ref()),
                            )
                        }
                        Job::Delete(service) => {
                            let result = repository.delete(service);
                            failed_deletes[service.index()] = result.is_err();
                            (service, result)
                        }
                        Job::Barrier(reply) => {
                            // Empty records are not re-saved by checkpoint. Retry
                            // only explicit failed deletions, never absent/unreadable records.
                            for service in [Service::Kgc, Service::Luna, Service::Kwic] {
                                if failed_deletes[service.index()] {
                                    let result = repository.delete(service);
                                    failed_deletes[service.index()] = result.is_err();
                                    failures[service.index()] =
                                        result.err().map(|e| format!("{}: {e}", service.name()));
                                }
                            }
                            let errors: Vec<_> = failures.iter().flatten().cloned().collect();
                            let _ = reply.send(if errors.is_empty() {
                                Ok(())
                            } else {
                                Err(errors.join("; "))
                            });
                            continue;
                        }
                    };
                    failures[service.index()] = result.err().map(|error| {
                        log::warn!("{} credential persistence pending: {error}", service.name());
                        format!("{}: {error}", service.name())
                    });
                }
            })
            .expect("start session storage worker");
        Self(send)
    }
    pub fn load(&self, service: Service) -> Loaded {
        let (send, receive) = mpsc::channel();
        self.0
            .send(Job::Load(service, send))
            .map_err(|e| StoreError::new("worker_unavailable", e.to_string()))?;
        receive
            .recv()
            .map_err(|e| StoreError::new("worker_unavailable", e.to_string()))?
    }
    pub fn save(
        &self,
        service: Service,
        cookies: Arc<reqwest_cookie_store::CookieStoreMutex>,
        identity: Option<AuthSession>,
    ) {
        if self.0.send(Job::Save(service, cookies, identity)).is_err() {
            log::error!("Session storage worker unavailable");
        }
    }
    pub fn delete(&self, service: Service) {
        if self.0.send(Job::Delete(service)).is_err() {
            log::error!("Session storage worker unavailable");
        }
    }
    pub fn barrier(&self) -> mpsc::Receiver<Result<(), String>> {
        let (send, receive) = mpsc::channel();
        let _ = self.0.send(Job::Barrier(send));
        receive
    }
}
