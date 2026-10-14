import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { Menu } from '@/services/api/emulator';

import { DsMenuBar } from './DsMenuBar';

const MENUS: Menu[] = [
  {
    label: 'System',
    items: [
      {
        kind: 'entry',
        label: 'Pause',
        action: { type: 'togglePause' },
        enabled: true,
        checked: false,
        radio: false,
        reason: null,
        shortcut: 'Ctrl+P',
        hint: null,
      },
      {
        kind: 'entry',
        label: 'RAM search',
        action: null,
        enabled: false,
        checked: null,
        radio: false,
        reason: 'RAM search is not implemented yet',
        shortcut: null,
        hint: null,
      },
      { kind: 'separator' },
      {
        kind: 'submenu',
        label: 'Screen gap',
        enabled: true,
        items: [
          {
            kind: 'entry',
            label: '8 px',
            action: { type: 'setGap', value: 8 },
            enabled: true,
            checked: true,
            radio: true,
            reason: null,
            shortcut: null,
            hint: null,
          },
        ],
      },
    ],
  },
];

describe('DsMenuBar', () => {
  it('dispatches the clicked entry', () => {
    const onAction = vi.fn();
    render(<DsMenuBar menus={MENUS} onAction={onAction} />);

    fireEvent.click(screen.getByText('System'));
    expect(screen.getByText('Ctrl+P')).toBeTruthy();
    fireEvent.click(screen.getByText('Pause'));
    expect(onAction).toHaveBeenCalledWith({ type: 'togglePause' });
  });

  it('opens submenus and keeps disabled entries inert', () => {
    const onAction = vi.fn();
    render(<DsMenuBar menus={MENUS} onAction={onAction} />);

    fireEvent.click(screen.getByText('System'));
    fireEvent.click(screen.getByText('RAM search'));
    expect(onAction).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText('Screen gap'));
    fireEvent.click(screen.getByText('8 px'));
    expect(onAction).toHaveBeenCalledWith({ type: 'setGap', value: 8 });
  });
});
