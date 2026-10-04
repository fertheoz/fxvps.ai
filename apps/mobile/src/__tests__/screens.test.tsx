import { MockTradingApi } from '@fxvps/trading-core';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react-native';
import type { ReactNode } from 'react';
import { Segmented } from '../components/ui';
import { SettingsProvider } from '../state/settings';
import { TradingProvider, useTrading } from '../state/trading';
import { Text } from 'react-native';

jest.mock('@react-native-async-storage/async-storage', () =>
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  require('@react-native-async-storage/async-storage/jest/async-storage-mock'),
);

function Probe() {
  const { connection, account, symbols, metrics } = useTrading();
  return (
    <Text testID="probe">
      {connection}|{account?.id ?? ''}|{symbols.length}|{metrics ? metrics.balance : ''}
    </Text>
  );
}

const wrap = (children: ReactNode, api: MockTradingApi) => (
  <SettingsProvider>
    <TradingProvider api={api}>{children}</TradingProvider>
  </SettingsProvider>
);

describe('providers', () => {
  afterEach(async () => {
    await cleanup();
  });

  it('connects to the mock api and exposes account metrics', async () => {
    jest.useRealTimers();
    const api = new MockTradingApi({ seed: 1, latencyMs: 0, tickIntervalMs: 0 });
    await render(wrap(<Probe />, api));
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });
    const text = screen.getByTestId('probe').props.children.join('');
    expect(text).toMatch(/^connected\|.+\|\d+\|\d+$/);
  });

  it('segmented control reports selection', async () => {
    const onChange = jest.fn();
    const api = new MockTradingApi({ seed: 1, latencyMs: 0, tickIntervalMs: 0 });
    await render(
      wrap(
        <Segmented
          options={[
            { value: 'a', label: 'A' },
            { value: 'b', label: 'B' },
          ]}
          value="a"
          onChange={onChange}
        />,
        api,
      ),
    );
    await fireEvent.press(screen.getByText('B'));
    expect(onChange).toHaveBeenCalledWith('b');
  });
});
