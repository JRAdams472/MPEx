import { defineConfig, globalIgnores } from "eslint/config";
import eslintReact from "@eslint-react/eslint-plugin";
import nextPlugin from "@next/eslint-plugin-next";
import reactHooks from "eslint-plugin-react-hooks";
import jsxA11yX from "eslint-plugin-jsx-a11y-x";
import importX from "eslint-plugin-import-x";
import globals from "globals";
import tseslint from "typescript-eslint";

// Flat config equivalent of eslint-config-next (which pins eslint-plugin-react,
// incompatible with ESLint 10):
//   typescript-eslint recommended        — was eslint-config-next/typescript
//   @eslint-react recommended-typescript — replaces eslint-plugin-react
//   eslint-plugin-react-hooks            — official hooks rules
//   @next/next recommended + core-web-vitals — Next.js rules
//   jsx-a11y-x / import-x                — ESLint-10-ready forks registered
//                                          under the legacy namespaces.
const eslintConfig = defineConfig([
  ...tseslint.configs.recommended,
  {
    rules: {
      "@typescript-eslint/no-unused-vars": ["warn", { ignoreRestSiblings: true }],
      "@typescript-eslint/no-unused-expressions": "warn",
    },
  },
  eslintReact.configs["recommended-typescript"],
  reactHooks.configs.flat["recommended-latest"],
  eslintReact.configs["disable-conflict-eslint-plugin-react-hooks"],
  {
    name: "mpex",
    files: ["**/*.{js,jsx,mjs,ts,tsx,mts,cts}"],
    plugins: {
      "@next/next": nextPlugin,
      "jsx-a11y": jsxA11yX,
      import: importX,
    },
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
    rules: {
      ...nextPlugin.configs.recommended.rules,
      "import/no-anonymous-default-export": "warn",
      "jsx-a11y/alt-text": ["warn", { elements: ["img"], img: ["Image"] }],
      "jsx-a11y/aria-props": "warn",
      "jsx-a11y/aria-proptypes": "warn",
      "jsx-a11y/aria-unsupported-elements": "warn",
      "jsx-a11y/role-has-required-aria-props": "warn",
      "jsx-a11y/role-supports-aria-props": "warn",
    },
  },
  {
    name: "mpex/next-core-web-vitals",
    rules: { ...nextPlugin.configs["core-web-vitals"].rules },
  },
  globalIgnores([".next/**", "out/**", "build/**", "next-env.d.ts", "coverage/**"]),
]);

export default eslintConfig;
