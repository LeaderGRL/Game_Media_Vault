import { type ReactNode, useEffect, useId, useRef, useState } from "react";

import type { Option } from "./catalog";
import { Icon } from "./icons";

/** Toggles picked from a list; an empty selection is « any », its own chip. */
export function ChipGroup({
  label,
  options,
  selected,
  onChange,
  anyLabel,
  more = [],
  moreLabel = "More",
}: {
  label: string;
  options: Option[];
  selected: string[];
  onChange: (selected: string[]) => void;
  /** Words of the chip standing for an empty selection. */
  anyLabel: string;
  /** Less common options, shown on request or once one of them is selected. */
  more?: Option[];
  moreLabel?: string;
}) {
  const [showingMore, setShowingMore] = useState(false);
  const moreShown =
    showingMore || more.some((option) => selected.includes(option.value));
  const shown = moreShown ? [...options, ...more] : options;
  return (
    <div className="chip-group" role="group" aria-label={label}>
      <button
        type="button"
        className="chip"
        aria-pressed={selected.length === 0}
        onClick={() => onChange([])}
      >
        {anyLabel}
      </button>
      {shown.map((option) => {
        const pressed = selected.includes(option.value);
        return (
          <button
            type="button"
            key={option.value}
            className="chip"
            aria-pressed={pressed}
            onClick={() =>
              onChange(
                pressed
                  ? selected.filter((value) => value !== option.value)
                  : [...selected, option.value],
              )
            }
          >
            {option.label}
          </button>
        );
      })}
      {more.length > 0 && !moreShown ? (
        <button type="button" className="chip more" onClick={() => setShowingMore(true)}>
          {moreLabel}
        </button>
      ) : null}
    </div>
  );
}

/** One choice among a few, as a row of buttons. */
export function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div className="segmented" role="group" aria-label={label}>
      {options.map((option) => (
        <button
          type="button"
          key={option.value}
          aria-pressed={option.value === value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/** A button opening a list of checkboxes, for filters picked from many values. */
export function FilterMenu({
  label,
  options,
  selected,
  onChange,
}: {
  label: string;
  options: Option[];
  selected: string[];
  onChange: (selected: string[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const menu = useRef<HTMLDivElement>(null);
  const popoverId = useId();

  useEffect(() => {
    if (!open) {
      return;
    }
    function closeOutside(event: MouseEvent) {
      if (!menu.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", closeOutside);
    return () => document.removeEventListener("mousedown", closeOutside);
  }, [open]);

  return (
    <div className="filter-menu" ref={menu}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={popoverId}
        onClick={() => setOpen((current) => !current)}
      >
        {label}
        {selected.length > 0 ? <span className="filter-count">{selected.length}</span> : null}
        <Icon name="chevron" size={16} />
      </button>
      {open ? (
        <div className="filter-popover" id={popoverId} role="group" aria-label={label}>
          {options.length === 0 ? <p className="hint">Nothing to filter yet.</p> : null}
          {options.map((option) => (
            <label className="choice" key={option.value}>
              <input
                type="checkbox"
                checked={selected.includes(option.value)}
                onChange={() =>
                  onChange(
                    selected.includes(option.value)
                      ? selected.filter((value) => value !== option.value)
                      : [...selected, option.value],
                  )
                }
              />
              {option.label}
            </label>
          ))}
          {selected.length > 0 ? (
            <div className="filter-popover-actions">
              <button type="button" className="ghost" onClick={() => onChange([])}>
                Clear
              </button>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** A labelled bar from 0 to 1; without a value it shows work under way. */
export function ProgressBar({
  label,
  value,
  detail,
  tone,
}: {
  label: string;
  value: number | null;
  detail?: ReactNode;
  tone?: "search";
}) {
  const percent = value === null ? null : Math.round(Math.min(Math.max(value, 0), 1) * 100);
  return (
    <div className="progress-block">
      <div className="progress-label">
        <span>
          <strong>{label}</strong>
          {percent === null ? null : ` · ${percent}%`}
        </span>
        {detail ? <span>{detail}</span> : null}
      </div>
      <div
        className={["progress", tone, percent === null ? "indeterminate" : null]
          .filter(Boolean)
          .join(" ")}
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
      >
        <span style={percent === null ? undefined : { width: `${percent}%` }} />
      </div>
    </div>
  );
}

/** A modal window over the app, closed with its button or the Escape key. */
export function Dialog({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  useEffect(() => {
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  return (
    <div
      className="dialog-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <section className="dialog" role="dialog" aria-modal="true" aria-label={title}>
        <div className="dialog-header">
          <h2>{title}</h2>
          <button type="button" className="ghost icon-button" aria-label="Close" onClick={onClose}>
            <Icon name="close" size={18} />
          </button>
        </div>
        {children}
      </section>
    </div>
  );
}
