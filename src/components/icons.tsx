// Minimal inline icon set (stroke icons, 16px grid).

import type { SVGProps } from "react";

type P = SVGProps<SVGSVGElement> & { size?: number };

function Svg({ size = 16, children, ...rest }: P & { children: React.ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      {...rest}
    >
      {children}
    </svg>
  );
}

export const IconPlay = (p: P) => (
  <Svg {...p}>
    <path d="M4.5 3.2v9.6L12.5 8z" fill="currentColor" stroke="none" />
  </Svg>
);
export const IconPause = (p: P) => (
  <Svg {...p}>
    <rect x="4" y="3" width="2.6" height="10" rx=".6" fill="currentColor" stroke="none" />
    <rect x="9.4" y="3" width="2.6" height="10" rx=".6" fill="currentColor" stroke="none" />
  </Svg>
);
export const IconStop = (p: P) => (
  <Svg {...p}>
    <rect x="4" y="4" width="8" height="8" rx="1" fill="currentColor" stroke="none" />
  </Svg>
);
export const IconRecord = (p: P) => (
  <Svg {...p}>
    <circle cx="8" cy="8" r="4.5" fill="currentColor" stroke="none" />
  </Svg>
);
export const IconRewind = (p: P) => (
  <Svg {...p}>
    <path d="M3.5 3.5v9" />
    <path d="M12.5 3.5v9L5.5 8z" fill="currentColor" stroke="none" />
  </Svg>
);
export const IconLoop = (p: P) => (
  <Svg {...p}>
    <path d="M3 7V6a2 2 0 0 1 2-2h7l-2-2M13 9v1a2 2 0 0 1-2 2H4l2 2" />
  </Svg>
);
export const IconMetronome = (p: P) => (
  <Svg {...p}>
    <path d="M6 2h4l2.5 12h-9z" />
    <path d="M8 10l4-6" />
  </Svg>
);
export const IconUndo = (p: P) => (
  <Svg {...p}>
    <path d="M5 4L2.5 6.5 5 9" />
    <path d="M2.5 6.5H10a3.5 3.5 0 0 1 0 7H7" />
  </Svg>
);
export const IconRedo = (p: P) => (
  <Svg {...p}>
    <path d="M11 4l2.5 2.5L11 9" />
    <path d="M13.5 6.5H6a3.5 3.5 0 0 0 0 7h3" />
  </Svg>
);
export const IconPlus = (p: P) => (
  <Svg {...p}>
    <path d="M8 3v10M3 8h10" />
  </Svg>
);
export const IconMinus = (p: P) => (
  <Svg {...p}>
    <path d="M3 8h10" />
  </Svg>
);
export const IconTrash = (p: P) => (
  <Svg {...p}>
    <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5" />
  </Svg>
);
export const IconBrowser = (p: P) => (
  <Svg {...p}>
    <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
    <path d="M6 2.5v11" />
  </Svg>
);
export const IconInspector = (p: P) => (
  <Svg {...p}>
    <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
    <path d="M10 2.5v11" />
  </Svg>
);
export const IconMixer = (p: P) => (
  <Svg {...p}>
    <path d="M4 2v12M8 2v12M12 2v12" />
    <rect x="2.5" y="8" width="3" height="2" rx=".5" fill="currentColor" />
    <rect x="6.5" y="4.5" width="3" height="2" rx=".5" fill="currentColor" />
    <rect x="10.5" y="10" width="3" height="2" rx=".5" fill="currentColor" />
  </Svg>
);
export const IconPiano = (p: P) => (
  <Svg {...p}>
    <rect x="2" y="3" width="12" height="10" rx="1" />
    <path d="M5 3v10M8 3v10M11 3v10" />
    <path d="M4 3v5h2V3M10 3v5h2V3" fill="currentColor" />
  </Svg>
);
export const IconSave = (p: P) => (
  <Svg {...p}>
    <path d="M3 3h8l2 2v8H3z" />
    <path d="M5 3v3h5V3M5 13V9h6v4" />
  </Svg>
);
export const IconFolder = (p: P) => (
  <Svg {...p}>
    <path d="M2 4.5V12a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1H8L6.5 3.5H3a1 1 0 0 0-1 1z" />
  </Svg>
);
export const IconSettings = (p: P) => (
  <Svg {...p}>
    <circle cx="8" cy="8" r="2" />
    <path d="M8 1.8v1.7M8 12.5v1.7M14.2 8h-1.7M3.5 8H1.8M12.4 3.6l-1.2 1.2M4.8 11.2l-1.2 1.2M12.4 12.4l-1.2-1.2M4.8 4.8L3.6 3.6" />
  </Svg>
);
export const IconExport = (p: P) => (
  <Svg {...p}>
    <path d="M8 10V2M5 5l3-3 3 3" />
    <path d="M3 9v4h10V9" />
  </Svg>
);
export const IconKeyboard = (p: P) => (
  <Svg {...p}>
    <rect x="1.5" y="4" width="13" height="8" rx="1.2" />
    <path d="M4 7h.01M6.5 7h.01M9 7h.01M11.5 7h.01M5 9.5h6" />
  </Svg>
);
export const IconMagnet = (p: P) => (
  <Svg {...p}>
    <path d="M4 3v5a4 4 0 0 0 8 0V3" />
    <path d="M4 5.5h2.5M9.5 5.5H12" />
  </Svg>
);
export const IconClose = (p: P) => (
  <Svg {...p}>
    <path d="M4 4l8 8M12 4l-8 8" />
  </Svg>
);
export const IconChevronDown = (p: P) => (
  <Svg {...p}>
    <path d="M4 6l4 4 4-4" />
  </Svg>
);
export const IconChevronRight = (p: P) => (
  <Svg {...p}>
    <path d="M6 4l4 4-4 4" />
  </Svg>
);
export const IconPower = (p: P) => (
  <Svg {...p}>
    <path d="M8 2v5" />
    <path d="M4.5 4.5a5 5 0 1 0 7 0" />
  </Svg>
);
export const IconUp = (p: P) => (
  <Svg {...p}>
    <path d="M4 10l4-4 4 4" />
  </Svg>
);
export const IconDown = (p: P) => (
  <Svg {...p}>
    <path d="M4 6l4 4 4-4" />
  </Svg>
);
export const IconMic = (p: P) => (
  <Svg {...p}>
    <rect x="6" y="2" width="4" height="7.5" rx="2" />
    <path d="M3.5 8a4.5 4.5 0 0 0 9 0M8 12.5V14" />
  </Svg>
);
export const IconWave = (p: P) => (
  <Svg {...p}>
    <path d="M1.5 8h1.5M4.5 5v6M7 3v10M9.5 6v4M12 4.5v7M14.5 8h0" />
  </Svg>
);
export const IconDrum = (p: P) => (
  <Svg {...p}>
    <ellipse cx="8" cy="6" rx="5.5" ry="2.2" />
    <path d="M2.5 6v4.5c0 1.2 2.5 2.2 5.5 2.2s5.5-1 5.5-2.2V6" />
    <path d="M10.5 1.5L8 5" />
  </Svg>
);
export const IconSynth = (p: P) => (
  <Svg {...p}>
    <path d="M1.5 9c1.5 0 1.5-5 3-5s1.5 8 3 8 1.5-6 3-6 1.5 3 3 3h1" />
  </Svg>
);
export const IconFx = (p: P) => (
  <Svg {...p}>
    <path d="M3 13l3-10M2.5 7h4M9 6l4 6M13 6l-4 6" />
  </Svg>
);
export const IconLoops = (p: P) => (
  <Svg {...p}>
    <rect x="2" y="3" width="5" height="4" rx="1" />
    <rect x="9" y="3" width="5" height="4" rx="1" />
    <rect x="2" y="9" width="5" height="4" rx="1" />
    <rect x="9" y="9" width="5" height="4" rx="1" />
  </Svg>
);
export const IconSearch = (p: P) => (
  <Svg {...p}>
    <circle cx="7" cy="7" r="4" />
    <path d="M10 10l3.5 3.5" />
  </Svg>
);
export const IconFile = (p: P) => (
  <Svg {...p}>
    <path d="M4 2h5l3 3v9H4z" />
    <path d="M9 2v3h3" />
  </Svg>
);
export const IconScissors = (p: P) => (
  <Svg {...p}>
    <circle cx="4.5" cy="11.5" r="2" />
    <circle cx="11.5" cy="11.5" r="2" />
    <path d="M5.8 10L12 2.5M10.2 10L4 2.5" />
  </Svg>
);
export const IconCopy = (p: P) => (
  <Svg {...p}>
    <rect x="5" y="5" width="8" height="8" rx="1" />
    <path d="M3 10.5V3h7.5" />
  </Svg>
);
export const IconHelp = (p: P) => (
  <Svg {...p}>
    <circle cx="8" cy="8" r="6" />
    <path d="M6.3 6.3a1.8 1.8 0 1 1 2.5 1.6c-.5.3-.8.7-.8 1.3M8 11.5h.01" />
  </Svg>
);
export const IconFollow = (p: P) => (
  <Svg {...p}>
    <path d="M8 2v12" />
    <path d="M3 6l-1.5 2L3 10M13 6l1.5 2L13 10" />
  </Svg>
);
export const IconDots = (p: P) => (
  <Svg {...p}>
    <circle cx="4" cy="8" r=".8" fill="currentColor" />
    <circle cx="8" cy="8" r=".8" fill="currentColor" />
    <circle cx="12" cy="8" r=".8" fill="currentColor" />
  </Svg>
);
export const IconSection = (p: P) => (
  <Svg {...p}>
    <path d="M2 4h5v3H2zM9 4h5v3H9z" />
    <path d="M2 10h12" />
  </Svg>
);
export const IconPencil = (p: P) => (
  <Svg {...p}>
    <path d="M10.5 2.5l3 3L6 13H3v-3z" />
  </Svg>
);
export const IconPointer = (p: P) => (
  <Svg {...p}>
    <path d="M4 2.5l8 5.5-3.5.8L10.5 13 9 13.7 7 9.5 4 12z" fill="currentColor" stroke="none" />
  </Svg>
);
