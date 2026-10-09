//! Isolated encoding/transfer benchmark using the production cache DTOs and
//! buffered codecs. No application, microphone, user files, storage or IPC.
use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::io::{self, Write};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/live/types.rs"]
mod types;
use types::*;
#[path = "support/allocation.rs"]
mod allocation;
#[allow(dead_code)]
#[path = "../src/atomic_file.rs"]
mod atomic_file;
#[path = "../src/live/markdown/fixtures.rs"]
mod fixtures;

mod live {
    use super::*;
    #[allow(dead_code)]
    mod schema {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/live/cache/types.rs"
        ));
    }
    use schema::{LiveDayCacheRef, LiveLineDeltaRef};
    #[allow(dead_code, unused_imports)]
    mod atomic {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/live/cache/atomic.rs"
        ));
    }
    mod encoding {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/live/cache/encoding.rs"
        ));
    }

    #[derive(Default)]
    struct ChecksumWriter {
        hash: Sha256,
        bytes: usize,
        transfers: usize,
    }
    impl Write for ChecksumWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.hash.update(bytes);
            self.bytes += bytes.len();
            self.transfers += 1;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[derive(Debug)]
    struct Checksum {
        hash: [u8; 32],
        bytes: usize,
        transfers: usize,
    }
    impl ChecksumWriter {
        fn finish(self) -> Checksum {
            Checksum {
                hash: self.hash.finalize().into(),
                bytes: self.bytes,
                transfers: self.transfers,
            }
        }
    }
    fn elapsed(build: impl FnOnce() -> Checksum) -> Duration {
        let started = Instant::now();
        let result = black_box(build());
        let elapsed = started.elapsed();
        black_box(result);
        elapsed
    }

    pub(super) fn run(lines: &[SharedTranscriptLine], chunks: &[SharedSummaryChunk], name: &str) {
        let cache = LiveDayCacheRef {
            date: "2026-10-08".into(),
            started_at: "2026-10-08 10:00:00".into(),
            course_name: name,
            transcript_lines: lines,
            summaries: chunks,
        };
        for journal in [false, true] {
            let old = || {
                // Removed accumulation paths: whole cache to_vec, or the
                // complete new NDJSON batch in one Vec, then one transfer.
                let mut bytes = Vec::new();
                if journal {
                    for (i, line) in lines.iter().enumerate() {
                        serde_json::to_writer(
                            &mut bytes,
                            &LiveLineDeltaRef {
                                i,
                                t: &line.text,
                                a: &line.at,
                            },
                        )
                        .unwrap();
                        bytes.push(b'\n');
                    }
                } else {
                    bytes = serde_json::to_vec(&cache).unwrap();
                }
                let mut target = ChecksumWriter::default();
                target.write_all(&bytes).unwrap();
                target.finish()
            };
            let new = || {
                let mut target = ChecksumWriter::default();
                if journal {
                    encoding::deltas(&mut target, lines, 0).unwrap();
                } else {
                    encoding::cache(&mut target, &cache).unwrap();
                }
                target.finish()
            };
            for _ in 0..3 {
                let old = old();
                let new = new();
                assert_eq!(old.bytes, new.bytes);
                assert_eq!(old.hash, new.hash);
            }
            let mut old_times = [Duration::ZERO; 9];
            let mut new_times = [Duration::ZERO; 9];
            for index in 0..9 {
                if index % 2 == 0 {
                    old_times[index] = elapsed(old);
                    new_times[index] = elapsed(new);
                } else {
                    new_times[index] = elapsed(new);
                    old_times[index] = elapsed(old);
                }
            }
            old_times.sort_unstable();
            new_times.sort_unstable();
            let (old_value, old_alloc) = allocation::tracked(old);
            let (new_value, new_alloc) = allocation::tracked(new);
            assert_eq!(old_value.bytes, new_value.bytes);
            assert_eq!(old_value.hash, new_value.hash);
            assert_eq!(old_alloc.live, 0);
            assert_eq!(new_alloc.live, 0);
            assert!(
                new_alloc.peak <= atomic::BUFFER_BYTES + 1024,
                "encoder retained additional row-dependent output"
            );
            println!("{} lines / {} / {} encoded bytes: {:.3} -> {:.3} ms; allocation requests {} -> {}; total requested bytes {} -> {}; peak requested live bytes {} -> {}; sink transfers {} -> {}",
                lines.len(),if journal {"journal"} else {"cache"},old_value.bytes,
                old_times[4].as_secs_f64()*1000.0,new_times[4].as_secs_f64()*1000.0,
                old_alloc.calls,new_alloc.calls,old_alloc.requested,new_alloc.requested,
                old_alloc.peak,new_alloc.peak,old_value.transfers,new_value.transfers);
        }
    }
}

fn main() {
    println!("Production DTOs/codecs; default dev profile; input setup excluded; 9 alternating runs after warmup.");
    println!("Encoding and SHA-256 counting sink only: no disk, sync/rename, tail repair, app, IPC/UI/RSS/GPU. Allocation tracked in separate calls, excludes inputs/allocator/realloc internals.");
    let course = fixtures::course(false);
    for count in [0, 1_000, 10_000, 50_000] {
        let lines = fixtures::lines(count);
        let chunks = fixtures::summaries((count / 200).max(1));
        live::run(&lines, &chunks, &course.course_name);
    }
}
