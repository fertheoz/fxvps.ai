import type { ReactNode } from 'react';
import { Pressable, StyleSheet, Text, View, type ViewStyle } from 'react-native';
import { useSettings } from '../state/settings';

export function Screen({ children }: { children?: ReactNode }) {
  const { palette } = useSettings();
  return <View style={[styles.screen, { backgroundColor: palette.bg }]}>{children}</View>;
}

export function Card({ children, style }: { children: ReactNode; style?: ViewStyle }) {
  const { palette } = useSettings();
  return (
    <View style={[styles.card, { backgroundColor: palette.card, borderColor: palette.border }, style]}>{children}</View>
  );
}

export function Label({ children, muted, color }: { children: ReactNode; muted?: boolean; color?: string }) {
  const { palette } = useSettings();
  return <Text style={{ color: color ?? (muted ? palette.muted : palette.text), fontSize: 14 }}>{children}</Text>;
}

export function Row({ label, value, color }: { label: string; value: string; color?: string }) {
  return (
    <View style={styles.row}>
      <Label muted>{label}</Label>
      <Label color={color}>{value}</Label>
    </View>
  );
}

export function Segmented<T extends string>({
  options,
  value,
  onChange,
}: {
  options: readonly { value: T; label: string; color?: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  const { palette } = useSettings();
  return (
    <View style={[styles.segmented, { borderColor: palette.border }]}>
      {options.map((o) => {
        const active = o.value === value;
        const bg = active ? (o.color ?? palette.accent) : 'transparent';
        return (
          <Pressable
            key={o.value}
            accessibilityRole="button"
            accessibilityState={{ selected: active }}
            onPress={() => onChange(o.value)}
            style={[styles.segment, { backgroundColor: bg }]}
          >
            <Text style={{ color: active ? '#fff' : palette.text, fontWeight: '600' }}>{o.label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

export function Button({
  title,
  onPress,
  color,
  disabled,
  testID,
}: {
  title: string;
  onPress: () => void;
  color?: string;
  disabled?: boolean;
  testID?: string;
}) {
  const { palette } = useSettings();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      disabled={disabled}
      onPress={onPress}
      style={[styles.button, { backgroundColor: color ?? palette.accent, opacity: disabled ? 0.5 : 1 }]}
    >
      <Text style={{ color: '#fff', fontWeight: '700' }}>{title}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 10, padding: 12, margin: 8 },
  row: { flexDirection: 'row', justifyContent: 'space-between', paddingVertical: 4 },
  segmented: { flexDirection: 'row', borderWidth: 1, borderRadius: 8, overflow: 'hidden', marginVertical: 6 },
  segment: { flex: 1, alignItems: 'center', paddingVertical: 8 },
  button: { borderRadius: 8, alignItems: 'center', paddingVertical: 12, marginVertical: 6 },
});
