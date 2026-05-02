import { expect, test } from "bun:test";

test("smoke: bun test runs", () => {
	expect(1 + 1).toBe(2);
});
