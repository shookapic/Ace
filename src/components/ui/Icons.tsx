interface IconProps {
  className?: string;
}

// Minimal 24×24 line icons, stroke-based, inherit currentColor. One consistent
// family so the toolbar reads as a set rather than an emoji grab-bag.
function base(children: React.ReactNode, className?: string) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export const PlusIcon = ({ className }: IconProps) =>
  base(
    <>
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </>,
    className
  );

export const HistoryIcon = ({ className }: IconProps) =>
  base(
    <>
      <path d="M3 3v5h5" />
      <path d="M3.05 13A9 9 0 1 0 6 5.3L3 8" />
      <path d="M12 7v5l3.5 2" />
    </>,
    className
  );

export const SettingsIcon = ({ className }: IconProps) =>
  base(
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </>,
    className
  );

export const PaperclipIcon = ({ className }: IconProps) =>
  base(
    <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />,
    className
  );

export const MicIcon = ({ className }: IconProps) =>
  base(
    <>
      <rect x="9" y="2" width="6" height="12" rx="3" />
      <path d="M5 10a7 7 0 0 0 14 0" />
      <path d="M12 19v3" />
    </>,
    className
  );

export const StopIcon = ({ className }: IconProps) =>
  base(<rect x="6" y="6" width="12" height="12" rx="2.5" fill="currentColor" stroke="none" />, className);

export const SendIcon = ({ className }: IconProps) =>
  base(
    <>
      <path d="M12 19V5" />
      <path d="M5 12l7-7 7 7" />
    </>,
    className
  );

export const CloseIcon = ({ className }: IconProps) =>
  base(
    <>
      <path d="M18 6 6 18" />
      <path d="M6 6l12 12" />
    </>,
    className
  );

export const ChevronDownIcon = ({ className }: IconProps) =>
  base(<path d="m6 9 6 6 6-6" />, className);

export const CheckIcon = ({ className }: IconProps) =>
  base(<path d="M20 6 9 17l-5-5" />, className);

export const CompareIcon = ({ className }: IconProps) =>
  base(
    <>
      <rect x="3" y="4" width="7" height="16" rx="1.5" />
      <rect x="14" y="4" width="7" height="16" rx="1.5" />
    </>,
    className
  );

export const CameraIcon = ({ className }: IconProps) =>
  base(
    <>
      <path d="M3 8a2 2 0 0 1 2-2h1.2a1 1 0 0 0 .8-.4l.9-1.2a1 1 0 0 1 .8-.4h4.6a1 1 0 0 1 .8.4l.9 1.2a1 1 0 0 0 .8.4H19a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      <circle cx="12" cy="13" r="3.2" />
    </>,
    className
  );

export const RegenerateIcon = ({ className }: IconProps) =>
  base(
    <>
      <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
      <path d="M21 3v5h-5" />
      <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
      <path d="M3 21v-5h5" />
    </>,
    className
  );

export const CopyIcon = ({ className }: IconProps) =>
  base(
    <>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </>,
    className
  );

export const PencilIcon = ({ className }: IconProps) =>
  base(
    <>
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </>,
    className
  );

export const SpinnerIcon = ({ className }: IconProps) =>
  base(
    <>
      <path d="M12 3a9 9 0 1 0 9 9" />
    </>,
    className
  );
