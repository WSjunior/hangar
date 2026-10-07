// @ts-check
const path = require('node:path');
const { getDefaultConfig } = require('expo/metro-config');

const raiz = path.resolve(__dirname, '..');

/** @type {import('expo/metro-config').MetroConfig} */
const config = getDefaultConfig(__dirname);

// O app NÃO é workspace da raiz (quem instala o Hangar não pode baixar o toolchain do RN), então a
// detecção automática de monorepo do Expo não vale aqui: sem estas duas linhas o Metro não vigia
// nem resolve o `@hangar/core`, que mora fora de mobile/ e entra por `file:`.
config.watchFolders = [path.resolve(raiz, 'packages/core')];
config.resolver.nodeModulesPaths = [
  path.resolve(__dirname, 'node_modules'),
  path.resolve(raiz, 'node_modules'),
];
// O npm deixa o expo-modules-core dentro de expo/node_modules (o reanimated exige um worklets mais
// novo que o aceito por ele, e o conflito impede subir o pacote), mas expo-audio e outros o importam
// direto. O mesmo caminho está nos `paths` do tsconfig.
config.resolver.extraNodeModules = {
  'expo-modules-core': path.resolve(__dirname, 'node_modules/expo/node_modules/expo-modules-core'),
};

module.exports = config;
