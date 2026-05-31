import { describe, expect, it } from "bun:test";
import { type DesktopStack, desktopStartOrder, desktopStopOrder } from "../src/lib/desktop.ts";

/**
 * Property test: FUSE-safety topology invariant — for BOTH desktop stacks.
 *
 * For every stack, the stop order MUST be the exact reverse of the start order,
 * and the XFCE session (the primary HOME-fd holder on the gocryptfs mount)
 * MUST be first in the stop order. This guarantees fusermount -u /workspace
 * never hits EBUSY regardless of which stack is built.
 */
const STACKS: DesktopStack[] = ["selkies", "kasmvnc"];

// The program that must die first on stop (primary HOME-fd holder) — XFCE for both.
const FIRST_TO_STOP: Record<DesktopStack, string> = {
  selkies: "desktop:desktop-xfce",
  kasmvnc: "desktop:desktop-xfce",
};

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
      // selkies: xvfb + audio + selkies + xfce = 4
      // kasmvnc: xvnc + xfce = 2 (Xkasmvnc is X server + WS server combined)
      const expected = stack === "selkies" ? 4 : 2;
      expect(desktopStartOrder(stack).length).toBe(expected);
      expect(desktopStopOrder(stack).length).toBe(expected);
    });
  }
});
