import { defineConfig } from 'vitest/config';
import path from 'node:path';

export default defineConfig({
  resolve: {
    alias: {
      // O app deixou de ser workspace da raiz, então react/react-dom vêm do node_modules daqui.
      react: path.resolve(__dirname, 'node_modules/react'),
      'react-dom': path.resolve(__dirname, 'node_modules/react-dom'),
      'react-native': path.resolve(__dirname, 'src/__mocks__/react-native.ts'),
      'react-native-unistyles': path.resolve(__dirname, 'src/__mocks__/react-native-unistyles.ts'),
      'react-native-webview': path.resolve(__dirname, 'src/__mocks__/react-native-webview.ts'),
      'expo-image': path.resolve(__dirname, 'src/__mocks__/expo-image.ts'),
      'react-native-reanimated': path.resolve(__dirname, 'src/__mocks__/react-native-reanimated.ts'),
      'react-native-worklets': path.resolve(__dirname, 'src/__mocks__/react-native-worklets.ts'),
      'react-native-svg': path.resolve(__dirname, 'src/__mocks__/react-native-svg.ts'),
      'expo-haptics': path.resolve(__dirname, 'src/__mocks__/expo-haptics.ts'),
      'react-native-safe-area-context': path.resolve(__dirname, 'src/__mocks__/react-native-safe-area-context.ts'),
      'react-native-gesture-handler': path.resolve(__dirname, 'src/__mocks__/react-native-gesture-handler.ts'),
    },
  },
  test: {
    // Externo, o lucide carregaria o react-native de verdade (Flow) e não passaria pelos aliases acima.
    server: { deps: { inline: ['lucide-react-native'] } },
    // Um fork por núcleo (o padrão do vitest) com mais de uma suíte rodando ao mesmo tempo
    // enche a RAM e joga a máquina em swap. Dois bastam pro tamanho desta suíte.
    maxWorkers: 2,
    environment: 'node',
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
    globals: true,
  },
});
