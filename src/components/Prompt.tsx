// Promise-based text prompt (window.prompt is not available in the webview).

import { useEffect, useRef, useState } from "react";
import { create } from "zustand";

interface PromptState {
  open: boolean;
  title: string;
  value: string;
  resolve: ((v: string | null) => void) | null;
}

const usePrompt = create<PromptState>(() => ({ open: false, title: "", value: "", resolve: null }));

export function promptText(title: string, value = ""): Promise<string | null> {
  return new Promise((resolve) => {
    usePrompt.setState({ open: true, title, value, resolve });
  });
}

export function PromptHost() {
  const { open, title, value, resolve } = usePrompt();
  const [text, setText] = useState(value);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (open) {
      setText(value);
      setTimeout(() => ref.current?.select(), 0);
    }
  }, [open, value]);
  if (!open) return null;
  const close = (v: string | null) => {
    usePrompt.setState({ open: false, resolve: null });
    resolve?.(v);
  };
  return (
    <div className="modal-backdrop" onMouseDown={() => close(null)}>
      <div className="modal prompt" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-title">{title}</div>
        <input
          ref={ref}
          className="text-input"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") close(text.trim() || null);
            if (e.key === "Escape") close(null);
          }}
        />
        <div className="modal-actions">
          <button className="btn" onClick={() => close(null)}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => close(text.trim() || null)}>
            OK
          </button>
        </div>
      </div>
    </div>
  );
}
