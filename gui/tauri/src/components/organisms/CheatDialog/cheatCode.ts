/**
 * Why `code` is not a valid Action Replay code, or `null` (same rules as
 * `ds_emu::parse_ar_code`: 8-digit hex words, an even number of them).
 */
export const cheatCodeError = (code: string): string | null => {
  const words = code.split(/\s+/).filter(Boolean);
  const bad = words.find((w) => !/^[0-9a-f]{8}$/i.test(w));
  if (bad !== undefined) return `"${bad}" is not an 8-digit hex number`;
  if (words.length % 2 !== 0) {
    return "The code must have an even number of words (address value pairs)";
  }
  return null;
};
