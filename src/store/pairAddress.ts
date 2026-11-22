// Pair a Mac, address entry: the check the pairing form makes before it calls
// `peer_pair`, so a typo is answered at once. A mirror of the backend's
// `peer_machines::parse_pair_address`, which stays the one that decides.
// What is typed is an SSH alias, `user@host`, a tailnet or dotted name or an
// IPv4 address, each with an optional `:port` (the port Hollow answers on
// there). Pairing only ever goes over the person's own ssh login to that
// name, so URLs, paths, IPv6 literals, this Mac's own loopback and
// half-written IPv4 numbers are refused here too.

export interface PairAddress { host: string; port: number | null }
export type PairAddressResult = { ok: true; address: PairAddress } | { ok: false; message: string };

const HOST_CHARS = /^[A-Za-z0-9._@-]+$/;

export function parsePairAddress(input: string): PairAddressResult {
  const typed = input.trim();
  const refuse = (why: string): PairAddressResult => ({ ok: false, message: `\`${typed}\` isn't an address Hollow can pair with: ${why}` });
  if (!typed) return { ok: false, message: 'Type the other Mac’s name or address.' };
  if (/:\/\/|[/?#]/.test(typed)) return refuse('type its name, not a web address or a path.');
  if (/\s/.test(typed)) return refuse('it has a space in it.');
  const at = typed.lastIndexOf('@');
  const user = at >= 0 ? typed.slice(0, at) : null;
  const rest = at >= 0 ? typed.slice(at + 1) : typed;
  if (user !== null && (!user || user.includes('@')) || !rest) return refuse('expected name, user@name or name:port.');
  let name = rest;
  let port: number | null = null;
  const colon = rest.indexOf(':');
  if (colon >= 0) {
    const tail = rest.slice(colon + 1);
    if (tail.includes(':')) return refuse('IPv6 addresses aren’t supported; use its name.');
    const n = /^[0-9]+$/.test(tail) ? Number(tail) : NaN;
    if (!(n >= 1 && n <= 65535)) return refuse('the port must be a number from 1 to 65535.');
    name = rest.slice(0, colon);
    port = n;
  }
  const bare = name.replace(/\.+$/, '');
  if (!bare || bare.split('.').some(part => !part)) return refuse('a name can’t have empty parts.');
  const lower = bare.toLowerCase();
  if (lower === 'localhost' || lower.endsWith('.localhost')) return refuse('that is this Mac, not another one.');
  if (/^[0-9.]+$/.test(bare)) {
    const parts = bare.split('.');
    // Like Rust's Ipv4Addr: four octets, 0 to 255, no leading zeros.
    const whole = parts.length === 4 && parts.every(p => p.length <= 3 && Number(p) <= 255 && (p === '0' || !p.startsWith('0')));
    if (!whole) return refuse('that isn’t a whole IPv4 address.');
    const [a = 0, b = 0, c = 0, d = 0] = parts.map(Number);
    if (a === 127 || (a === 0 && b === 0 && c === 0 && d === 0)) return refuse('that is this Mac, not another one.');
    if (a >= 224) return refuse('that isn’t a Mac’s address.');
  }
  const host = user !== null ? `${user}@${name}` : name;
  if (host.startsWith('-') || !HOST_CHARS.test(host)) {
    return refuse('use letters, digits, dots, dashes, underscores and @, not starting with a dash.');
  }
  return { ok: true, address: { host, port } };
}
