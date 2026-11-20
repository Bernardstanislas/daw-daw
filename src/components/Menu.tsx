// Dropdown + context menus.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { create } from "zustand";

export interface MenuItem {
  label?: string;
  shortcut?: string;
  onClick?: () => void;
  disabled?: boolean;
  checked?: boolean;
  danger?: boolean;
  separator?: boolean;
  submenu?: MenuItem[];
}

interface ContextMenuState {
  open: boolean;
  x: number;
  y: number;
  items: MenuItem[];
}

const useContextMenu = create<ContextMenuState>(() => ({ open: false, x: 0, y: 0, items: [] }));

export function openContextMenu(e: { clientX: number; clientY: number; preventDefault?: () => void }, items: MenuItem[]) {
  e.preventDefault?.();
  useContextMenu.setState({ open: true, x: e.clientX, y: e.clientY, items });
}

export function closeContextMenu() {
  useContextMenu.setState({ open: false });
}

function MenuList({ items, onClose }: { items: MenuItem[]; onClose: () => void }) {
  const [sub, setSub] = useState<number | null>(null);
  return (
    <div className="menu-list" onContextMenu={(e) => e.preventDefault()}>
      {items.map((it, i) =>
        it.separator ? (
          <div key={i} className="menu-sep" />
        ) : (
          <div
            key={i}
            className={`menu-item ${it.disabled ? "disabled" : ""} ${it.danger ? "danger" : ""}`}
            onMouseEnter={() => setSub(it.submenu ? i : null)}
            onClick={(e) => {
              e.stopPropagation();
              if (it.disabled || it.submenu) return;
              onClose();
              it.onClick?.();
            }}
          >
            <span className="menu-check">{it.checked ? "✓" : ""}</span>
            <span className="menu-label">{it.label}</span>
            {it.shortcut && <span className="menu-shortcut">{it.shortcut}</span>}
            {it.submenu && <span className="menu-shortcut">›</span>}
            {it.submenu && sub === i && (
              <SubMenu items={it.submenu} onClose={onClose} />
            )}
          </div>
        ),
      )}
    </div>
  );
}

/** Submenu that opens to the right, flipping left / shifting up to stay inside the window. */
function SubMenu({ items, onClose }: { items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<React.CSSProperties>({ visibility: "hidden" });
  useLayoutEffect(() => {
    const el = ref.current;
    const parent = el?.parentElement;
    if (!el || !parent) return;
    const p = parent.getBoundingClientRect();
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const m = 6;
    const s: React.CSSProperties = {};
    if (p.right + w > window.innerWidth - m) {
      s.left = "auto";
      s.right = "100%";
      s.paddingLeft = 0;
      s.paddingRight = 4;
    }
    const top = p.top - 5;
    if (top + h > window.innerHeight - m) s.top = Math.max(m, window.innerHeight - m - h) - p.top;
    setStyle(s);
  }, [items]);
  return (
    <div className="menu-sub" ref={ref} style={style}>
      <MenuList items={items} onClose={onClose} />
    </div>
  );
}

export function ContextMenuHost() {
  const { open, x, y, items } = useContextMenu();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  useLayoutEffect(() => {
    if (!open || !ref.current) return;
    const r = ref.current.getBoundingClientRect();
    setPos({
      x: Math.min(x, window.innerWidth - r.width - 6),
      y: Math.min(y, window.innerHeight - r.height - 6),
    });
  }, [open, x, y, items]);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (ref.current && ref.current.contains(e.target as Node)) return;
      closeContextMenu();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && closeContextMenu();
    window.addEventListener("mousedown", close, true);
    window.addEventListener("keydown", esc, true);
    window.addEventListener("blur", closeContextMenu);
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("keydown", esc, true);
      window.removeEventListener("blur", closeContextMenu);
    };
  }, [open]);
  if (!open) return null;
  return (
    <div className="context-menu" ref={ref} style={{ left: pos.x, top: pos.y }}>
      <MenuList items={items} onClose={closeContextMenu} />
    </div>
  );
}

/** A button that opens a dropdown menu below itself. */
export function MenuButton({ className, children, items, title }: { className?: string; children: React.ReactNode; items: () => MenuItem[]; title?: string }) {
  return (
    <button
      className={className}
      title={title}
      onMouseDown={(e) => {
        e.stopPropagation();
        const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
        if (useContextMenu.getState().open) {
          closeContextMenu();
          return;
        }
        openContextMenu({ clientX: r.left, clientY: r.bottom + 4 }, items());
      }}
    >
      {children}
    </button>
  );
}
