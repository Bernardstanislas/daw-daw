// Pointer handlers for a key / row name that plays its note while held.
// Only the left button plays (a right-click opens a menu that swallows the
// pointerup), and the pointer is captured so the note always gets its note-off.

import { liveNote } from "../../lib/engine";
import { capturePointer } from "../../lib/music";

export function auditionProps(pitch: number, trackId: string) {
  const stop = () => liveNote(pitch, 0, false, trackId);
  return {
    onPointerDown: (e: React.PointerEvent<HTMLElement>) => {
      if (e.button !== 0) return;
      capturePointer(e.currentTarget, e.pointerId);
      liveNote(pitch, 0.8, true, trackId);
    },
    onPointerUp: stop,
    onPointerCancel: stop,
    onLostPointerCapture: stop,
  };
}
