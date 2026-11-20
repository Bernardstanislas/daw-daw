// Step input controls (shared by the piano roll and the step view).

import { setStepLength, startStepRecord, stepBack, stepRest, STEP_LENGTHS, stopStepRecord, useStepRecord } from "../../lib/stepRecord";

/** `pattern`: in the step view, the step length is the view's own and the cursor wraps around the pattern. */
export function StepInputControls({ clipId, pattern = false }: { clipId: string; pattern?: boolean }) {
  const { active, clipId: recClip, step, cursor, wrapAt } = useStepRecord();
  const on = active && recClip === clipId;
  const pos = Math.round(cursor / step) + 1;
  return (
    <div className="step-input">
      <button
        className={`btn small ${on ? "live" : ""}`}
        onClick={() => (on ? stopStepRecord() : startStepRecord(clipId))}
        title="Step input: play notes on your MIDI keyboard (or musical typing), one step at a time"
      >
        <span className={`lamp ${on ? "lit rec" : ""}`} /> Step input
      </button>
      {on && (
        <>
          {!pattern && (
            <select value={step} onChange={(e) => setStepLength(+e.target.value)} title="Step length">
              {STEP_LENGTHS.map((s) => (
                <option key={s.label} value={s.beats}>
                  {s.label}
                </option>
              ))}
            </select>
          )}
          <div className="seg" role="group" aria-label="Step input cursor">
            <button onClick={stepBack} title="Back one step (←)">
              ◀
            </button>
            <span className="seg-label" title="Step being written">
              {pattern && wrapAt > 0 ? `${pos}/${Math.round(wrapAt / step)}` : pos}
            </span>
            <button onClick={stepRest} title="Forward one step, leaving it as it is (→)">
              ▶
            </button>
          </div>
        </>
      )}
    </div>
  );
}
