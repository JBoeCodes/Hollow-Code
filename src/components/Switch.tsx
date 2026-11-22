/** An on/off switch (role="switch"). Styles live in styles/connectors-library.css,
 *  which the Settings connectors section loads. */
export function Switch(props: { checked: boolean; label: string; disabled?: boolean; onChange: (next: boolean) => void }) {
  return <button
    type="button"
    role="switch"
    class="hollow-switch"
    aria-checked={props.checked}
    aria-label={props.label}
    disabled={props.disabled}
    onClick={e => { e.stopPropagation(); props.onChange(!props.checked); }}
  ><span class="hollow-switch-knob" /></button>;
}
