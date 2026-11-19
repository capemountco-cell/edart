/** An on/off switch with its state spelled out. At least 44px tall. */
export function Switch({
  on,
  label,
  disabled,
  onChange,
}: {
  on: boolean;
  /** What the switch controls, for screen readers. */
  label: string;
  disabled?: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className="switch"
      disabled={disabled}
      onClick={() => onChange(!on)}
    >
      <span className="switch-track" aria-hidden="true">
        <span className="switch-thumb" />
      </span>
      <span className="switch-label">{on ? "On" : "Off"}</span>
    </button>
  );
}
