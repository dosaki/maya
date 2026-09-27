/**
 * Protects against the board repainting under the cursor: clicks landing
 * within `windowMs` of a repaint are ignored (the card under the pointer may
 * have just moved), and repaints are deferred while a pointer button is down.
 */
export interface ClickGuard {
  markPaint(nowMs: number): void;
  allowClick(nowMs: number): boolean;
  setPointerDown(down: boolean): void;
  canPaint(): boolean;
}

export function makeClickGuard(windowMs = 300): ClickGuard {
  let lastPaint = Number.NEGATIVE_INFINITY;
  let pointerDown = false;
  return {
    markPaint: (nowMs) => {
      lastPaint = nowMs;
    },
    allowClick: (nowMs) => nowMs - lastPaint > windowMs,
    setPointerDown: (down) => {
      pointerDown = down;
    },
    canPaint: () => !pointerDown,
  };
}
