// https://docs.expo.dev/guides/using-eslint/
const { defineConfig } = require('eslint/config');
const expoConfig = require('eslint-config-expo/flat');

module.exports = defineConfig([
  expoConfig,
  { files: ['*.config.js'], languageOptions: { globals: { __dirname: 'readonly', require: 'readonly', module: 'writable' } } },
  { ignores: ['dist/*', 'node_modules/*', '.expo/*'] },
]);
