export interface DoctorResult {
  component: string;
  status: "ok" | "warn" | "fail";
  detail?: string;
}
