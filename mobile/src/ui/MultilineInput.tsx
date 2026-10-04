import { forwardRef, useState, type ComponentType } from 'react';
import { TextInput, TurboModuleRegistry, type TextInputProps } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import type { PasteInputProps, PastedFile } from '@mattermost/react-native-paste-input';

// O campo nativo do React Native só cola texto; o PasteInput entrega também imagem e arquivo
// colados. O módulo nativo dele só existe a partir do build que o inclui: importar a lib num build
// antigo derruba o app ao abrir, então sem o módulo o campo segue o TextInput de sempre.
const PasteInput: ComponentType<PasteInputProps> | null = TurboModuleRegistry.get('PasteInputModule')
  ? require('@mattermost/react-native-paste-input').default
  : null;

type Props = TextInputProps & {
  maxHeight?: number;
  mono?: boolean;
  /** Arquivos colados no campo; só chega com o PasteInput no build. */
  onPaste?: (files: PastedFile[], error: string | null) => void;
};

// Campo multilinha do app: cresce até `maxHeight` e daí rola. Fundo é do contêiner (o composer
// carrega o vidro), por isso o input não pinta nada.
export const MultilineInput = forwardRef<TextInput, Props>(
  function MultilineInput({ maxHeight = 120, mono, style, onContentSizeChange, onPaste, ...rest }, ref) {
    const { theme } = useUnistyles();
    // Texto posto por código (ditado, rascunho) não faz o iOS crescer o campo sozinho: a linha de
    // baixo ficava cortada. A altura passa a seguir o conteúdo medido.
    const [contentH, setContentH] = useState(0);
    const props = {
      ref,
      multiline: true,
      scrollEnabled: true,
      placeholderTextColor: theme.tokens.text.muted,
      onContentSizeChange: (e: Parameters<NonNullable<TextInputProps['onContentSizeChange']>>[0]) => {
        setContentH(e.nativeEvent.contentSize.height);
        onContentSizeChange?.(e);
      },
      style: [
        styles.input,
        { maxHeight, color: theme.tokens.text.primary },
        contentH > 0 && { height: Math.min(maxHeight, contentH + 16) },
        mono && { fontFamily: theme.base.fontMono },
        style,
      ],
      ...rest,
    };
    if (onPaste && PasteInput) {
      return <PasteInput {...props} onPaste={(error, files) => onPaste(files, error ?? null)} />;
    }
    return <TextInput {...props} />;
  },
);

const styles = StyleSheet.create((theme) => ({
  input: { fontSize: theme.base.text.base, paddingVertical: 8, paddingHorizontal: 10, textAlignVertical: 'top' },
}));
