// Step input controls (shared by the piano roll and the step view).

import { setStepLength, startStepRecord, stepBack, stepRest, STEP_LENGTHS, stopStepRecord, useStepRecord } from "../../lib/stepRecord";

export function StepInputControls({ clipId }: { clipId: string }) {
  const { active, clipId: recClip, step } = useStepRecord();
  const on = active && recClip === clipId;
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
          <select value={step} onChange={(e) => setStepLength(+e.target.value)} title="Step length">
            {STEP_LENGTHS.map((s) => (
              <option key={s.label} value={s.beats}>
                {s.label}
              </option>
            ))}
          </select>
          <button className="btn small" onClick={stepBack} title="Back one step (←)">
            Back
          </button>
          <button className="btn small" onClick={stepRest} title="Rest: skip one step (→)">
            Rest
          </button>
        </>
      )}
    </div>
  );
}
