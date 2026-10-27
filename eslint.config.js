import js from '@eslint/js'
import jsxA11y from 'eslint-plugin-jsx-a11y'
import reactHooks from 'eslint-plugin-react-hooks'
import globals from 'globals'
import tseslint from 'typescript-eslint'

// Lints the shared packages under packages/*. ui/ and apps/depths/web keep their own configs,
// which ESLint finds first for their files; the rules here match theirs.
export default tseslint.config(
  { ignores: ['**/node_modules', '**/dist', 'ui', 'apps', 'target'] },
  {
    files: ['packages/**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      ...tseslint.configs.strictTypeChecked,
      reactHooks.configs.flat['recommended-latest'],
      jsxA11y.flatConfigs.recommended,
    ],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
      parserOptions: {
        projectService: { allowDefaultProject: ['packages/theme/vitest.config.ts'] },
        tsconfigRootDir: import.meta.dirname },
    },
    rules: {
      '@typescript-eslint/restrict-template-expressions': ['error', { allowNumber: true }],
      // `() => set(...)` and `onClick={() => doThing()}` are idiomatic in React and Zustand.
      '@typescript-eslint/no-confusing-void-expression': ['error', { ignoreArrowShorthand: true }],
      '@typescript-eslint/consistent-type-imports': ['error', { fixStyle: 'inline-type-imports' }],
    },
  },
)
