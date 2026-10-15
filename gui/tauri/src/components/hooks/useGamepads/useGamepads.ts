import { useEffect, useRef, useState } from 'react';

import { gamepadKeys, gamepadSpeedHeld } from '@/services/api/emulator';

/**
 * Polls the connected gamepads once per animation frame (Web Gamepad API)
 * and calls `onKeys` with the pressed DS button bits whenever they change.
 *
 * `onSpeed` is called once each time the left stick is pressed in (it steps
 * the emulation speed).
 *
 * Returns the names of the connected gamepads. Browsers only expose a pad
 * after one of its buttons was pressed while the page has focus.
 */
export const useGamepads = (onKeys: (keys: number) => void, onSpeed?: () => void) => {
  const [names, setNames] = useState<string[]>([]);
  const onKeysRef = useRef(onKeys);
  const onSpeedRef = useRef(onSpeed);
  useEffect(() => {
    onKeysRef.current = onKeys;
    onSpeedRef.current = onSpeed;
  });

  useEffect(() => {
    let frame = 0;
    let lastKeys = 0;
    let lastNames = '';
    let speedHeld = false;

    const poll = () => {
      const pads = navigator.getGamepads?.() ?? [];

      const keys = gamepadKeys(pads);
      if (keys !== lastKeys) {
        lastKeys = keys;
        onKeysRef.current(keys);
      }

      const held = gamepadSpeedHeld(pads);
      if (held && !speedHeld) onSpeedRef.current?.();
      speedHeld = held;

      const connected = pads.flatMap((pad) => (pad?.connected ? [pad.id] : []));
      const joined = connected.join('\n');
      if (joined !== lastNames) {
        lastNames = joined;
        setNames(connected);
      }

      frame = requestAnimationFrame(poll);
    };

    frame = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(frame);
  }, []);

  return names;
};
