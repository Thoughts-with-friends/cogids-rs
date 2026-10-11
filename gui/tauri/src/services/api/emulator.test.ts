import { describe, expect, it } from 'vitest';

import { KEY, gamepadKeys } from './emulator';

/** A "standard" mapping gamepad with the given buttons held. */
const pad = (pressed: number[], axes: number[] = [0, 0, 0, 0]): Gamepad =>
  ({
    id: 'Xbox One Game Controller',
    connected: true,
    mapping: 'standard',
    axes,
    buttons: Array.from({ length: 17 }, (_, i) => ({
      pressed: pressed.includes(i),
      touched: pressed.includes(i),
      value: pressed.includes(i) ? 1 : 0,
    })),
  }) as unknown as Gamepad;

describe('gamepadKeys', () => {
  it('maps face buttons by position like the DS', () => {
    expect(gamepadKeys([pad([1])])).toBe(KEY.A); // right
    expect(gamepadKeys([pad([0])])).toBe(KEY.B); // bottom
    expect(gamepadKeys([pad([3])])).toBe(KEY.X); // top
    expect(gamepadKeys([pad([2])])).toBe(KEY.Y); // left
  });

  it('maps shoulders and triggers to L/R, and START/SELECT', () => {
    expect(gamepadKeys([pad([4])])).toBe(KEY.L);
    expect(gamepadKeys([pad([6])])).toBe(KEY.L);
    expect(gamepadKeys([pad([5, 7])])).toBe(KEY.R);
    expect(gamepadKeys([pad([8, 9])])).toBe(KEY.SELECT | KEY.START);
  });

  it('maps the D-pad and the left stick', () => {
    expect(gamepadKeys([pad([12, 15])])).toBe(KEY.UP | KEY.RIGHT);
    expect(gamepadKeys([pad([], [-0.9, 0.9])])).toBe(KEY.LEFT | KEY.DOWN);
    expect(gamepadKeys([pad([], [0.3, -0.3])])).toBe(0); // dead zone
  });

  it('combines pads and ignores empty / disconnected slots', () => {
    const gone = { ...pad([0]), connected: false } as Gamepad;
    expect(gamepadKeys([null, pad([1]), gone, pad([9])])).toBe(KEY.A | KEY.START);
  });
});
