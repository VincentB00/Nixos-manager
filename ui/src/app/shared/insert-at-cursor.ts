/** Insert `text` at the caret of a textarea, replacing any selection, and
 *  return the new value. Keeps focus and places the caret after the insert. */
export function insertAtCursor(el: HTMLTextAreaElement, text: string): string {
  const start = el.selectionStart ?? el.value.length;
  const end = el.selectionEnd ?? start;
  const value = el.value.slice(0, start) + text + el.value.slice(end);
  el.value = value;
  const caret = start + text.length;
  el.focus();
  el.setSelectionRange(caret, caret);
  return value;
}
