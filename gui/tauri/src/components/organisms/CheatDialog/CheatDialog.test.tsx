import { fireEvent, render, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const setCheats = vi.fn().mockResolvedValue(undefined);

vi.mock('@/services/api/emulator', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/api/emulator')>();
  return {
    ...actual,
    EMU: {
      getCheats: vi.fn().mockResolvedValue([
        { name: '60 FPS', memo: 'note', code: '02000000 00000001', enabled: true, kind: 'Action Replay' },
        { name: 'Max money', memo: '', code: '02000004 0001869F', enabled: false, kind: 'Action Replay' },
      ]),
      setCheats: (...args: unknown[]) => setCheats(...args),
      romInfo: vi.fn().mockRejectedValue('no info'),
    },
  };
});

import { CheatDialog } from './CheatDialog';
import { cheatCodeError } from './cheatCode';

describe('cheatCodeError', () => {
  it('accepts address / value pairs', () => {
    expect(cheatCodeError('60D4BF48 00000000\nb0d4bf48 00000000')).toBeNull();
    expect(cheatCodeError('')).toBeNull();
  });

  it('explains bad codes', () => {
    expect(cheatCodeError('XXXXXXXX 00000000')).toMatch(/XXXXXXXX/);
    expect(cheatCodeError('00000000')).toMatch(/even/);
  });
});

describe('CheatDialog', () => {
  it('lists the cheats and edits the selected one', async () => {
    const onClose = vi.fn();
    const { findAllByText, getAllByRole, getByLabelText, getByText } = render(
      <CheatDialog onClose={onClose} />,
    );
    expect(await findAllByText('Action Replay')).toHaveLength(2);
    expect((getByLabelText('Name') as HTMLInputElement).value).toBe('60 FPS');

    // Pick the second cheat, rename it, save, enable it, OK.
    fireEvent.click(getByText('Max money'));
    fireEvent.change(getByLabelText('Name'), { target: { value: 'Money' } });
    fireEvent.click(getByText('Save'));
    fireEvent.click(getAllByRole('checkbox')[1]);
    fireEvent.click(getByText('OK'));

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(setCheats).toHaveBeenCalledWith([
      expect.objectContaining({ name: '60 FPS', enabled: true }),
      expect.objectContaining({ name: 'Money', enabled: true }),
    ]);
  });

  it('blocks saving an invalid code', async () => {
    const { findAllByText, getByLabelText, getByText } = render(<CheatDialog onClose={vi.fn()} />);
    await findAllByText('Action Replay');
    fireEvent.change(getByLabelText('Code'), { target: { value: '1234' } });
    expect(getByText(/not an 8-digit hex number/)).toBeTruthy();
    expect(getByText('Save').closest('button')?.disabled).toBe(true);
  });
});
