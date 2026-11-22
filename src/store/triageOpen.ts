// Incoming was cut (overnight 11-08 S3). The openers stay so older callers
// compile, and they never open a screen. Filing lives in chat, not in Needs you.

export const triageOpen = () => false;
export const openTriage = () => false;
export const closeTriage = () => { /* no screen to close */ };
