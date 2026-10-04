const { withAppDelegate, withPodfile } = require('expo/config-plugins');

// O @mattermost/react-native-paste-input precisa que o AppDelegate do iOS chame
// PasteInputModule.setup com a rootViewFactory; sem isso a colagem de arquivo não chega ao JS.
// A lib não traz plugin do Expo, e o ios/ é gerado a cada build: a linha entra aqui.
// O header dela puxa C++ do codegen, que o Swift não importa: a chamada vai pelo runtime do ObjC.
const SETUP = `
    if let pasteInput = NSClassFromString("PasteInputModule") {
      _ = (pasteInput as AnyObject).perform(NSSelectorFromString("setup:"), with: factory.rootViewFactory)
    } else {
      NSLog("PasteInputModule ausente: colar arquivo no campo de texto não vai funcionar")
    }`;
const ANCHOR = /(factory\.startReactNative\([\s\S]*?launchOptions: launchOptions\))/;

// O podspec da lib recusa instalar sem RCT_NEW_ARCH_ENABLED=1, mas o RN atual só tem a arquitetura
// nova e não define mais a variável: sem ela o pod install falha.
const NEW_ARCH = "ENV['RCT_NEW_ARCH_ENABLED'] = '1'";

const withAppDelegateSetup = (config) => withAppDelegate(config, (cfg) => {
  const { language, contents } = cfg.modResults;
  if (language !== 'swift') throw new Error('withPasteInput: AppDelegate em Swift esperado, veio ' + language);
  if (contents.includes('NSClassFromString("PasteInputModule")')) return cfg;
  if (!ANCHOR.test(contents)) throw new Error('withPasteInput: factory.startReactNative não encontrado no AppDelegate');
  cfg.modResults.contents = contents.replace(ANCHOR, `$1${SETUP}`);
  return cfg;
});

const withNewArchEnv = (config) => withPodfile(config, (cfg) => {
  if (!cfg.modResults.contents.includes(NEW_ARCH)) cfg.modResults.contents = `${NEW_ARCH}\n${cfg.modResults.contents}`;
  return cfg;
});

module.exports = (config) => withNewArchEnv(withAppDelegateSetup(config));
