import { describe, expect, it } from "bun:test";
import {
  type DesktopTarget,
  desktopStartOrder,
  desktopStopOrder,
} from "../src/lib/desktop.ts";

/**
 * Property test: FUSE-safety topology invariant — for BOTH desktop stacks.
 *
 * For every stack, the stop order MUST be the exact reverse of the start order,
 * and the XFCE session (the primary HOME-fd holder on the gocryptfs mount)
 * MUST be first in the stop order. This guarantees fusermount -u /workspace
 * never hits EBUSY regardless of which stack(s) are running (selkies, vnc, or
 * both in parallel).
 */
const STACKS: DesktopTarget[] = ["selkies", "vnc"];

// The program that must die first on stop (primary HOME-fd holder) — XFCE per stack.
const FIRST_TO_STOP: Record<DesktopTarget, string> = {
  selkies: "desktop:desktop-xfce",
  vnc: "vnc:desktop-xfce-vnc",
};

// Expected program count per stack.
//   selkies: xvfb + audio + selkies + xfce = 4
//   vnc:     xvnc + xfce-vnc = 2 (Xkasmvnc is X server + WS server combined)
const PROGRAM_COUNT: Record<DesktopTarget, number> = { selkies: 4, vnc: 2 };

describe("Desktop topology FUSE-safety invariant (both stacks)", () => {
  for (const stack of STACKS) {
    it(`[${stack}] STOP_ORDER is the exact reverse of START_ORDER`, () => {
      const reversed = [...desktopStartOrder(stack)].reverse();
      expect(desktopStopOrder(stack)).toEqual(reversed);
    });

    it(`[${stack}] primary HOME-fd holder (XFCE) is first in STOP_ORDER`, () => {
      expect(desktopStopOrder(stack)[0]).toBe(FIRST_TO_STOP[stack]);
    });

    it(`[${stack}] START_ORDER has correct program count`, () => {
      expect(desktopStartOrder(stack).length).toBe(PROGRAM_COUNT[stack]);
      expect(desktopStopOrder(stack).length).toBe(PROGRAM_COUNT[stack]);
    });
  }

  it("selkies and vnc use independent supervisord groups", () => {
    // No program name is shared between the two stacks → independent start/stop.
    const selkiesProgs = new Set(desktopStartOrder("selkies"));
    const vncProgs = desktopStartOrder("vnc");
    for (const p of vncProgs) {
      expect(selkiesProgs.has(p)).toBe(false);
    }
  });
});
