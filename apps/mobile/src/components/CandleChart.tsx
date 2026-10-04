import type { Bar } from '@fxvps/trading-core';
import { useState } from 'react';
import { View, type LayoutChangeEvent } from 'react-native';
import Svg, { Line, Rect } from 'react-native-svg';
import { layoutCandles } from '../lib/candles';
import { useSettings } from '../state/settings';

/** Lightweight SVG candlestick chart (works on iOS, Android and web). */
export function CandleChart({ bars, height = 280 }: { bars: readonly Bar[]; height?: number }) {
  const { palette } = useSettings();
  const [width, setWidth] = useState(0);
  const layout = layoutCandles(bars, width, height);
  const onLayout = (e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width);
  return (
    <View onLayout={onLayout} style={{ height, backgroundColor: palette.card }} testID="candle-chart">
      {width > 0 && (
        <Svg width={width} height={height}>
          {layout.candles.map((c, i) => {
            const color = c.up ? palette.up : palette.down;
            const cx = c.x + c.width / 2;
            return [
              <Line key={`w${i}`} x1={cx} x2={cx} y1={c.wickTop} y2={c.wickBottom} stroke={color} strokeWidth={1} />,
              <Rect key={`b${i}`} x={c.x} y={c.bodyTop} width={c.width} height={c.bodyHeight} fill={color} />,
            ];
          })}
        </Svg>
      )}
    </View>
  );
}
