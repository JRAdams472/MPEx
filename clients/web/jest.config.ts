import type { Config } from "jest";
import nextJest from "next/jest.js";

const createJestConfig = nextJest({ dir: "./" });

const config: Config = {
  testEnvironment: "jest-environment-jsdom",
  testMatch: ["**/__tests__/**/*.test.{ts,tsx}"],
};

export default createJestConfig(config);
