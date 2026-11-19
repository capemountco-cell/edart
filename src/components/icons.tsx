/** Line icons, drawn to the design's 24px grid. */
import type { ReactNode } from "react";

function Icon({
  children,
  size = 20,
  weight = 1.75,
}: {
  children: ReactNode;
  size?: number;
  weight?: number;
}) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={weight}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  );
}

type Props = { size?: number };

export const PhoneIcon = ({ size }: Props) => (
  <Icon size={size}>
    <rect x="7" y="2.5" width="10" height="19" rx="2.5" />
    <path d="M11 18.5h2" />
  </Icon>
);
export const PhonesIcon = ({ size }: Props) => (
  <Icon size={size}>
    <rect x="4" y="3" width="9" height="17" rx="2" />
    <path d="M16 7h4M16 12h4M16 17h4" />
  </Icon>
);
export const ShieldIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M12 3l7 3v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6z" />
  </Icon>
);
export const ConnectorsIcon = ({ size }: Props) => (
  <Icon size={size}>
    <circle cx="6" cy="6" r="2" />
    <circle cx="18" cy="6" r="2" />
    <circle cx="12" cy="18" r="2" />
    <path d="M7.5 7.5l3.5 8.5M16.5 7.5L13 16" />
  </Icon>
);
export const SlidersIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M4 7h10M18 7h2M4 17h4M12 17h8" />
    <circle cx="16" cy="7" r="2" />
    <circle cx="10" cy="17" r="2" />
  </Icon>
);
export const HomeIcon = ({ size = 22 }: Props) => (
  <Icon size={size}>
    <path d="M4 11l8-7 8 7v9h-5v-6H9v6H4z" />
  </Icon>
);
export const KeyboardIcon = ({ size = 22 }: Props) => (
  <Icon size={size}>
    <rect x="2.5" y="6" width="19" height="12" rx="2" />
    <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10" />
  </Icon>
);
export const MoreIcon = ({ size = 22 }: Props) => (
  <Icon size={size} weight={3}>
    <path d="M5 12h.01M12 12h.01M19 12h.01" />
  </Icon>
);
export const PlusIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M12 5v14M5 12h14" />
  </Icon>
);
export const SendIcon = ({ size }: Props) => (
  <Icon size={size} weight={2.25}>
    <path d="M12 19V5M6 11l6-6 6 6" />
  </Icon>
);
export const StopIcon = ({ size }: Props) => (
  <Icon size={size} weight={2.25}>
    <rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" />
  </Icon>
);
export const MenuIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M4 7h16M4 12h16M4 17h16" />
  </Icon>
);
export const CloseIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M6 6l12 12M18 6L6 18" />
  </Icon>
);
export const ChevronIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M9 6l6 6-6 6" />
  </Icon>
);
export const CheckIcon = ({ size }: Props) => (
  <Icon size={size} weight={2.25}>
    <path d="M5 12.5l4.5 4.5L19 7.5" />
  </Icon>
);
export const CameraIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M4 8h3l1.5-2h7L17 8h3v11H4z" />
    <circle cx="12" cy="13" r="3.25" />
  </Icon>
);
export const BluetoothIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M7 7l10 10-5 4V3l5 4L7 17" />
  </Icon>
);
export const CableIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M9 3v5M15 3v5M7 8h10v4a5 5 0 01-10 0zM12 17v4" />
  </Icon>
);
export const AlertIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M12 4l9 16H3zM12 10v4M12 17h.01" />
  </Icon>
);
export const TrendIcon = ({ size }: Props) => (
  <Icon size={size}>
    <path d="M3.5 17 9 11.5l3.5 3.5 8-8" />
    <path d="M15 7h5.5v5.5" />
  </Icon>
);
