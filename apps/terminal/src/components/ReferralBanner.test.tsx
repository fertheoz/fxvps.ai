import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('../api/clientApi', async (importOriginal) => {
  const real = await importOriginal<typeof import('../api/clientApi')>();
  return { ...real, clientApi: { ...real.clientApi, ibLink: vi.fn() } };
});

import { clientApi } from '../api/clientApi';
import { ReferralBanner } from './ReferralBanner';

const ibLink = vi.mocked(clientApi.ibLink);

describe('<ReferralBanner />', () => {
  beforeEach(() => {
    localStorage.clear();
    ibLink.mockReset();
  });

  it('shows nothing without a remembered code', () => {
    const { container } = render(<ReferralBanner account="DEMO-1" />);
    expect(container).toBeEmptyDOMElement();
  });

  it('asks first and links only on the button', async () => {
    localStorage.setItem('fxvps.ref', 'NINE');
    ibLink.mockResolvedValue({ ok: true });
    const { unmount } = render(<ReferralBanner account="DEMO-1" />);
    expect(screen.getByTestId('ref-banner')).toHaveTextContent('NINE');
    expect(screen.getByTestId('ref-banner')).toHaveTextContent('DEMO-1');
    expect(ibLink).not.toHaveBeenCalled();
    await userEvent.setup().click(screen.getByTestId('ref-accept'));
    expect(ibLink).toHaveBeenCalledWith('DEMO-1', 'NINE');
    await vi.waitFor(() => expect(screen.queryByTestId('ref-banner')).toBeNull());
    unmount();
  });

  it('"No thanks" drops the code and sends nothing', async () => {
    localStorage.setItem('fxvps.ref', 'NINE');
    render(<ReferralBanner account="DEMO-1" />);
    await userEvent.setup().click(screen.getByTestId('ref-dismiss'));
    expect(screen.queryByTestId('ref-banner')).toBeNull();
    expect(localStorage.getItem('fxvps.ref')).toBeNull();
    expect(ibLink).not.toHaveBeenCalled();
  });
});
