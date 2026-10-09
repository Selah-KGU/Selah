/** Native university-session contract. These functions only project evidence;
 * recovery decisions and cooldowns belong to the Rust manager. */
export type UniversityService = "kgc" | "luna" | "kwic";
export type SessionHealth = "unverified" | "valid" | "refreshing" | "needs_login" | "unavailable" | "signed_out";
export type RecoveryTrigger = "manual" | "request_failure" | "automatic_request" | "startup" | "foreground" | "background" | "keepalive";
export interface ServiceStatus {
  service: UniversityService;
  state: SessionHealth;
  credentials_present: boolean;
  last_verified_at: number | null;
  last_checked_at: number | null;
  last_attempt_at: number | null;
}
export interface UniversitySnapshot {
  generation: number;
  revision: number;
  signed_out: boolean;
  login_persistence_pending?: boolean;
  services: ServiceStatus[];
}
export interface UniversityIdentity {
  username: string;
  display_name: string;
  student_id: string;
  faculty: string;
  department: string;
}
export interface RecoveryReport {
  snapshot: UniversitySnapshot;
  identity: UniversityIdentity | null;
  results: {
    service: UniversityService;
    outcome: "verified" | "needs_login" | "unavailable" | "deferred" | "signed_out";
    recovered: boolean;
    retry_at: number | null;
    message: string | null;
  }[];
}
export type SessionError =
  | { kind: "cancelled" | "needs_login" }
  | { kind: "invalid_service" | "unavailable"; message: string }
  | { kind: "storage"; message: { kind: string; message: string } };

export function serviceVerified(report: RecoveryReport, service: UniversityService): boolean {
  return report.results.some(result => result.service === service && result.outcome === "verified");
}
export function projectUniversitySession(snapshot: UniversitySnapshot) {
  const usable = (service: UniversityService) => !snapshot.signed_out && snapshot.services.some(status =>
    status.service === service && status.credentials_present && status.last_verified_at !== null);
  return {
    luna: usable("luna"),
    kwic: usable("kwic"),
    needsLogin: !snapshot.signed_out && snapshot.services.some(status =>
      status.service !== "kgc" && status.state === "needs_login"),
  };
}
