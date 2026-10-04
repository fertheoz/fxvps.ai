const path = require('path');

/** @type {import('jest').Config} */
module.exports = {
  preset: 'jest-expo',
  roots: ['<rootDir>/src', '<rootDir>/app'],
  // trading-core lives outside this package and has no node_modules of its own.
  modulePaths: ['<rootDir>/node_modules'],
  moduleNameMapper: {
    '^@fxvps/trading-core$': path.resolve(__dirname, '../../packages/trading-core/src/index.ts'),
    '^big\\.js$': path.resolve(__dirname, 'node_modules/big.js'),
  },
  transformIgnorePatterns: [
    'node_modules/(?!((jest-)?react-native|@react-native(-community)?)|expo(nent)?|@expo(nent)?/.*|@expo-google-fonts/.*|react-navigation|@react-navigation/.*|react-native-svg|big\\.js)',
  ],
};
