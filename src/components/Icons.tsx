import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function IconBase({ children, ...props }: IconProps) {
  return (
    <svg
      aria-hidden="true"
      fill="none"
      height="20"
      viewBox="0 0 24 24"
      width="20"
      {...props}
    >
      {children}
    </svg>
  );
}

const stroke = {
  stroke: "currentColor",
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  strokeWidth: 1.8,
};

export function FolderIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path {...stroke} d="M3.75 6.75h5l2 2h9.5v8.5a2 2 0 0 1-2 2H5.75a2 2 0 0 1-2-2v-10.5Z" />
      <path {...stroke} d="M3.75 9h16.5" />
    </IconBase>
  );
}

export function TranscriptIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path {...stroke} d="M6.25 3.75h8l3.5 3.5v13H6.25a2 2 0 0 1-2-2V5.75a2 2 0 0 1 2-2Z" />
      <path {...stroke} d="M14.25 3.75v4h3.5M8 12h6.5M8 15.5h5M8 8.5h2.5" />
    </IconBase>
  );
}

export function SettingsIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <circle {...stroke} cx="12" cy="12" r="3" />
      <path
        {...stroke}
        d="m19 13.7 1.1 1.8-2.1 2.1-1.8-1.1a7.7 7.7 0 0 1-2 .8l-.5 2.1h-3.4l-.5-2.1a7.7 7.7 0 0 1-2-.8L6 17.6l-2.1-2.1L5 13.7a7.8 7.8 0 0 1 0-3.4L3.9 8.5 6 6.4l1.8 1.1a7.7 7.7 0 0 1 2-.8l.5-2.1h3.4l.5 2.1a7.7 7.7 0 0 1 2 .8L18 6.4l2.1 2.1-1.1 1.8a7.8 7.8 0 0 1 0 3.4Z"
      />
    </IconBase>
  );
}

export function UploadIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path {...stroke} d="M12 15.5V4.5M8 8.5l4-4 4 4M5 13.5v4.75a1.75 1.75 0 0 0 1.75 1.75h10.5A1.75 1.75 0 0 0 19 18.25V13.5" />
    </IconBase>
  );
}

export function LinkIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path {...stroke} d="m9.5 14.5 5-5M8 16l-1 1a3.5 3.5 0 0 1-5-5l3-3a3.5 3.5 0 0 1 5 0M16 8l1-1a3.5 3.5 0 1 1 5 5l-3 3a3.5 3.5 0 0 1-5 0" />
    </IconBase>
  );
}

export function MicrophoneIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <rect {...stroke} height="10" rx="3" width="6" x="9" y="3" />
      <path {...stroke} d="M6.5 11.5a5.5 5.5 0 0 0 11 0M12 17v4M9 21h6" />
    </IconBase>
  );
}

export function PlayIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <circle {...stroke} cx="12" cy="12" r="9" />
      <path {...stroke} d="m10 8.5 5 3.5-5 3.5v-7Z" />
    </IconBase>
  );
}

export function ChevronRightIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path {...stroke} d="m9 5 7 7-7 7" />
    </IconBase>
  );
}

export function SunIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <circle {...stroke} cx="12" cy="12" r="3.5" />
      <path {...stroke} d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M19.1 4.9l-1.4 1.4M6.3 17.7l-1.4 1.4" />
    </IconBase>
  );
}

export function MoonIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path {...stroke} d="M20 15.2A8.6 8.6 0 0 1 8.8 4a8.7 8.7 0 1 0 11.2 11.2Z" />
    </IconBase>
  );
}

export function CheckIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path {...stroke} d="m5 12.5 4.3 4.2L19 7" />
    </IconBase>
  );
}

export function AlertIcon(props: IconProps) {
  return (
    <IconBase {...props}>
      <path {...stroke} d="M12 3 2.8 19h18.4L12 3Z" />
      <path {...stroke} d="M12 9v4M12 16.5v.1" />
    </IconBase>
  );
}
