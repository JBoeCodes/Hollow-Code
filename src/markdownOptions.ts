/**
 * marked's options per surface (P1-18). A file renders as written: a single
 * line break inside a paragraph is a space, as in the Library reader, the
 * preview window and Skills. Chat replies and other short messages keep
 * each typed line break.
 */
export const NOTE_MARKDOWN = { gfm: true, breaks: false, async: false } as const;
const MESSAGE_MARKDOWN = { gfm: true, breaks: true, async: false } as const;

export const markdownOptions = (breaks: boolean) => breaks ? MESSAGE_MARKDOWN : NOTE_MARKDOWN;
