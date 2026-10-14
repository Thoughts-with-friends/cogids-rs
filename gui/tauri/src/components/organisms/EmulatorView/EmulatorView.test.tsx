import { render, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@/services/api/dialog', () => ({ openPath: vi.fn() }));
vi.mock('@/services/api/emulator', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/api/emulator')>();
  return {
    ...actual,
    EMU: {
      getConfig: vi.fn().mockResolvedValue({
        romDir: '',
        selectedRom: null,
        scale: 2,
        layout: 'vertical',
        rotation: 0,
        gap: 8,
        swap: false,
        sizing: 'even',
        integerScaling: false,
        filtering: false,
        showFps: true,
        volume: 50,
        limitFramerate: true,
        recentRoms: [],
      }),
      setConfig: vi.fn().mockResolvedValue(undefined),
      menu: vi.fn().mockResolvedValue([{ label: 'File', items: [] }]),
      menuAction: vi.fn().mockResolvedValue(undefined),
      layout: vi.fn().mockResolvedValue({ screens: [null, null], natural_w: 256, natural_h: 392 }),
      status: vi.fn().mockResolvedValue({
        running: false,
        paused: false,
        fps: 0,
        rom: null,
        instanceDir: '',
        instance: 1,
      }),
      setKeys: vi.fn().mockResolvedValue(undefined),
      setTouch: vi.fn().mockResolvedValue(undefined),
      frame: vi.fn().mockResolvedValue(null),
    },
  };
});

import { EmulatorView } from './EmulatorView';

describe('EmulatorView', () => {
  it('gives both screens a 256x192 canvas bitmap', async () => {
    const { container } = render(<EmulatorView />);
    await waitFor(() => expect(container.querySelectorAll('canvas')).toHaveLength(2));

    for (const canvas of container.querySelectorAll('canvas')) {
      // A wrong bitmap size crops and stretches the frame (putImageData
      // writes 256x192 pixels).
      expect(canvas.width).toBe(256);
      expect(canvas.height).toBe(192);
      expect(canvas.style.aspectRatio).toBe('256 / 192');
    }
  });

  it('shows the menu bar from the backend', async () => {
    const { findByText } = render(<EmulatorView />);
    expect(await findByText('File')).toBeTruthy();
  });
});
