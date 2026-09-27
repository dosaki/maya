/**
 * Keeps the board from repainting under the cursor. Repaints are deferred
 * while a pointer button is down or the pointer has moved over the board in
 * the last `settleMs`; as a last resort, a click landing within
 * `clickWindowMs` of a repaint is ignored because the card under it may
 * have just moved.
 */
export interface ClickGuard {
  markPaint(nowMs: number): void;
  allowClick(nowMs: number): boolean;
  setPointerDown(down: boolean): void;
  markPointerMove(nowMs: number): void;
  markPointerLeave(): void;
  canPaint(nowMs: number): boolean;
}

export function makeClickGuard(opts: { clickWindowMs: number; settleMs: number } = { clickWindowMs: 120, settleMs: 300 }): ClickGuard {
  let lastPaint = Number.NEGATIVE_INFINITY;
  let lastMove = Number.NEGATIVE_INFINITY;
  let pointerDown = false;
  return {
    markPaint: (nowMs) => {
      lastPaint = nowMs;
    },
    allowClick: (nowMs) => nowMs - lastPaint > opts.clickWindowMs,
    setPointerDown: (down) => {
      pointerDown = down;
    },
    markPointerMove: (nowMs) => {
      lastMove = nowMs;
    },
    markPointerLeave: () => {
      lastMove = Number.NEGATIVE_INFINITY;
    },
    canPaint: (nowMs) => !pointerDown && nowMs - lastMove > opts.settleMs,
  };
}
