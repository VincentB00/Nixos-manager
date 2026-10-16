/**
 * Spans from the daemon are UTF-8 byte offsets (rnix); JavaScript strings are
 * indexed by UTF-16 code units. Any non-ASCII character before a node shifts
 * the two apart, so slicing a string directly by a span returns the wrong
 * text. These helpers convert.
 */

/** UTF-16 index of the character that starts at UTF-8 byte offset `byteOffset`. */
export function byteToIndex(source: string, byteOffset: number): number {
  let bytes = 0;
  let i = 0;
  while (i < source.length && bytes < byteOffset) {
    const cp = source.codePointAt(i)!;
    bytes += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
    i += cp >= 0x10000 ? 2 : 1;
  }
  return i;
}

/** `source` between two UTF-8 byte offsets. */
export function sliceBytes(source: string, start: number, end: number): string {
  const s = byteToIndex(source, start);
  return source.slice(s, s + (byteToIndex(source.slice(s), end - start)));
}
