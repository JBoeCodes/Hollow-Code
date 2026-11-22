/**
 * A small provider mark for an agent, used by the composer's model menu
 * (and, for `chatgpt`, by imported ChatGPT conversations).
 * Simple geometric glyphs in each provider's colour, not official logos.
 */
export function AgentMark(props: { agent: string; size?: number }) {
  const size = () => props.size ?? 16;
  return (
    <span class="agent-mark" data-agent={props.agent} aria-hidden="true" style={{ width: `${size()}px`, height: `${size()}px` }}>
      <svg viewBox="0 0 16 16" width={size()} height={size()}>
        {props.agent === 'claude' ? (
          <path fill="#D97757" d="M8 1.5l1.2 4.1 4.1-1.8-2.5 3.7 3.7 1.3-4 .9 1.6 3.9-3.3-2.6L8 14.5 7.2 11l-3.3 2.6 1.6-3.9-4-.9 3.7-1.3-2.5-3.7 4.1 1.8z" />
        ) : props.agent === 'codex' ? (
          <>
            <rect x="1" y="2" width="14" height="12" rx="3.5" fill="#6D7CFF" />
            <path d="M4.5 6l2 2-2 2M8 10.2h3.5" stroke="#fff" stroke-width="1.4" fill="none" stroke-linecap="round" stroke-linejoin="round" />
          </>
        ) : props.agent === 'antigravity' ? (
          <>
            <path fill="#4F8DF7" d="M8 1.8l6 11.4H2z" />
            <circle cx="8" cy="9.6" r="1.8" fill="#fff" />
          </>
        ) : props.agent === 'grok' ? (
          <>
            <rect x="1" y="1" width="14" height="14" rx="3.5" fill="currentColor" opacity="0.9" />
            <path d="M4.2 11.8l7.6-7.6M9.2 11.8l2.6-2.6" stroke="var(--canvas, #fff)" stroke-width="1.6" stroke-linecap="round" />
          </>
        ) : props.agent === 'chatgpt' ? (
          // ChatGPT (session importers): a hexagonal knot, not the logo.
          <>
            <path d="M8 1.6l5.5 3.2v6.4L8 14.4l-5.5-3.2V4.8z" fill="none" stroke="#10A37F" stroke-width="1.6" stroke-linejoin="round" />
            <path d="M8 4.6l2.9 1.7v3.4L8 11.4 5.1 9.7V6.3z" fill="#10A37F" />
          </>
        ) : (
          <circle cx="8" cy="8" r="6" fill="currentColor" opacity="0.5" />
        )}
      </svg>
    </span>
  );
}
