import { RUN_DIR } from "../shared/layout.ts";

/** Runtime state directory (non-FUSE, container-local). Loss on restart is non-fatal. */
export const BACKUP_STATE_DIR = `${RUN_DIR}/backup-state`;
