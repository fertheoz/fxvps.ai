// Metro config: resolve the shared pure-TS package from ../../packages/trading-core
// without a workspace install (mobile is installed with --ignore-workspace).
const path = require('path');
const { getDefaultConfig } = require('expo/metro-config');

const projectRoot = __dirname;
const coreRoot = path.resolve(projectRoot, '../../packages/trading-core');

const config = getDefaultConfig(projectRoot);
config.watchFolders = [...(config.watchFolders ?? []), coreRoot];
config.resolver.nodeModulesPaths = [
  ...(config.resolver.nodeModulesPaths ?? []),
  path.resolve(projectRoot, 'node_modules'),
];
config.resolver.extraNodeModules = {
  ...(config.resolver.extraNodeModules ?? {}),
  '@fxvps/trading-core': path.join(coreRoot, 'src'),
};
config.resolver.resolveRequest = (context, moduleName, platform) => {
  if (moduleName === '@fxvps/trading-core') {
    return { type: 'sourceFile', filePath: path.join(coreRoot, 'src/index.ts') };
  }
  return context.resolveRequest(context, moduleName, platform);
};

module.exports = config;
