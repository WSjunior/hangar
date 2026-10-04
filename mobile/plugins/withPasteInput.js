const { withAppDelegate } = require('expo/config-plugins');

// O @mattermost/react-native-paste-input precisa que o AppDelegate do iOS chame
// PasteInputModule.setup com a rootViewFactory; sem isso a colagem de arquivo não chega ao JS.
// A lib não traz plugin do Expo, e o ios/ é gerado a cada build: a linha entra aqui.
const IMPORT = 'import react_native_paste_input';
const ANCHOR = /(factory\.startReactNative\([\s\S]*?launchOptions: launchOptions\))/;

module.exports = function withPasteInput(config) {
  return withAppDelegate(config, (cfg) => {
    const { language, contents } = cfg.modResults;
    if (language !== 'swift') throw new Error('withPasteInput: AppDelegate em Swift esperado, veio ' + language);
    if (contents.includes('PasteInputModule.setup')) return cfg;
    if (!ANCHOR.test(contents)) throw new Error('withPasteInput: factory.startReactNative não encontrado no AppDelegate');
    cfg.modResults.contents = contents
      .replace('import React\n', `import React\n${IMPORT}\n`)
      .replace(ANCHOR, '$1\n    PasteInputModule.setup(factory.rootViewFactory)');
    return cfg;
  });
};
