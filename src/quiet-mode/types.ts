export interface QuietModeStatus {
  active: boolean;
  remaining_secs: number | null;
  total_secs: number | null;
}
