// Arranger lane: song sections (Intro / Verse / Chorus...). Sections can be
// drawn, renamed, resized and dragged to re-order the song (content follows).

import { memo, useState } from "react";
import { beginGesture, commit, endGesture, gesture, select, useStore } from "../../lib/store";
import {
  addSection,
  deleteSection,
  deleteSectionMarker,
  duplicateSection,
  moveSection,
  selectSectionContent,
  updateSection,
} from "../../lib/actions";
import { beatsPerBar, capturePointer } from "../../lib/music";
import { beatAtClientX } from "./geom";
import { openContextMenu, type MenuItem } from "../Menu";
import { promptText } from "../Prompt";
import type { Section } from "../../lib/types";
import { seek } from "../../lib/engine";

const COLORS = ["#6f8fb3", "#b58a5c", "#7f9e74", "#9a83b0", "#b3a05e", "#6e9f9a", "#ad7680"];

function sectionMenu(s: Section): MenuItem[] {
  return [
    {
      label: "Rename…",
      onClick: async () => {
        const n = await promptText("Section name", s.name);
        if (n) updateSection(s.id, (x) => void (x.name = n));
      },
    },
    {
      label: "Loop this section",
      onClick: () => {
        commit((p) => {
          p.loop = { enabled: true, start: s.start, end: s.start + s.length };
        });
        seek(s.start);
      },
    },
    { label: "Select content", onClick: () => selectSectionContent(s.id) },
    { separator: true },
    { label: "Duplicate section (insert copy after)", onClick: () => duplicateSection(s.id) },
    { label: "Delete section and its content", danger: true, onClick: () => deleteSection(s.id) },
    { label: "Remove marker only", onClick: () => deleteSectionMarker(s.id) },
    { separator: true },
    { label: "Color", submenu: COLORS.map((c) => ({ label: `● ${c}`, onClick: () => updateSection(s.id, (x) => void (x.color = c)) })) },
  ];
}

export const ArrangerLane = memo(function ArrangerLane({ width, ppb }: { width: number; ppb: number }) {
  const project = useStore((s) => s.project);
  const selectedId = useStore((s) => s.selection.sectionId);
  const bpb = beatsPerBar(project.timeSigNum, project.timeSigDen);
  const [drag, setDrag] = useState<{ id: string; x: number; to: number } | null>(null);
  const [draft, setDraft] = useState<{ start: number; end: number } | null>(null);

  const snapBar = (b: number) => Math.max(0, Math.round(b / bpb) * bpb);

  const onEmptyDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const el = e.currentTarget as HTMLElement;
    capturePointer(el, e.pointerId);
    const a = Math.floor(beatAtClientX(e.clientX) / bpb) * bpb;
    setDraft({ start: a, end: a + bpb });
    select({ sectionId: null });
    let cur = { start: a, end: a + bpb };
    const move = (ev: PointerEvent) => {
      const b = beatAtClientX(ev.clientX);
      const s = Math.min(a, Math.floor(b / bpb) * bpb);
      const en = Math.max(a + bpb, Math.ceil(b / bpb) * bpb);
      cur = { start: s, end: en };
      setDraft(cur);
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      setDraft(null);
      // avoid overlapping an existing section
      const overlaps = useStore.getState().project.sections.some((s) => s.start < cur.end && s.start + s.length > cur.start);
      if (!overlaps) addSection(cur.start, cur.end - cur.start);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  const onSectionDown = (e: React.PointerEvent, s: Section) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    select({ sectionId: s.id });
    const el = e.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const resize = e.clientX > rect.right - 6;
    capturePointer(el, e.pointerId);
    const b0 = beatAtClientX(e.clientX);
    let started = false;
    let to = s.start;
    if (resize) beginGesture();
    const move = (ev: PointerEvent) => {
      const b = beatAtClientX(ev.clientX);
      if (resize) {
        const end = Math.max(s.start + bpb, snapBar(b));
        gesture((p) => {
          const x = p.sections.find((x) => x.id === s.id);
          if (x) x.length = end - s.start;
        });
        return;
      }
      if (!started && Math.abs(b - b0) * ppb < 4) return;
      started = true;
      // Candidate destinations: section boundaries in the collapsed timeline, else bars.
      const others = useStore.getState().project.sections.filter((x) => x.id !== s.id);
      const collapsed = (x: number) => (x >= s.start + s.length ? x - s.length : x);
      const cands = new Set<number>([0]);
      for (const o of others) {
        cands.add(collapsed(o.start));
        cands.add(collapsed(o.start + o.length));
      }
      const want = s.start + (b - b0);
      let best = snapBar(want);
      let bestD = bpb * 1.01;
      for (const c of cands) {
        const d = Math.abs(c - want);
        if (d < bestD) {
          bestD = d;
          best = c;
        }
      }
      to = best;
      setDrag({ id: s.id, x: want, to });
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      if (resize) {
        endGesture();
        return;
      }
      setDrag(null);
      if (started) moveSection(s.id, to);
      else if (e.detail < 2) selectSectionContent(s.id);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  };

  return (
    <div className="arranger-lane" style={{ width }} onPointerDown={onEmptyDown} title="Drag to create a song section">
      {project.sections.map((s) => (
        <div
          key={s.id}
          className={`section ${selectedId === s.id ? "selected" : ""} ${drag?.id === s.id ? "dragging" : ""}`}
          style={{ left: s.start * ppb, width: s.length * ppb, ["--sec-color" as string]: s.color }}
          onPointerDown={(e) => onSectionDown(e, s)}
          onDoubleClick={async (e) => {
            e.stopPropagation();
            const n = await promptText("Section name", s.name);
            if (n) updateSection(s.id, (x) => void (x.name = n));
          }}
          onContextMenu={(e) => {
            e.stopPropagation();
            select({ sectionId: s.id });
            openContextMenu(e, sectionMenu(s));
          }}
          title={`${s.name} — drag to move (song re-orders), right-click for options`}
        >
          <span>{s.name}</span>
          <div className="section-resize" />
        </div>
      ))}
      {drag && (
        <>
          <div className="section ghost" style={{ left: drag.x * ppb, width: (project.sections.find((x) => x.id === drag.id)?.length ?? 0) * ppb }} />
          <div className="section-drop-marker" style={{ left: drag.to * ppb }} />
        </>
      )}
      {draft && <div className="section draft" style={{ left: draft.start * ppb, width: (draft.end - draft.start) * ppb }} />}
    </div>
  );
});
