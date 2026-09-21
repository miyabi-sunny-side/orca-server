import { expect, test } from "vitest";
import { LatestRequest } from "./latest-request";

test("a newer selection and disposal invalidate pending preview reads", () => {
  const requests = new LatestRequest();
  const slow = requests.begin();
  expect(slow.isCurrent()).toBe(true);
  const fast = requests.begin();
  expect(slow.isCurrent()).toBe(false);
  expect(fast.isCurrent()).toBe(true);
  requests.invalidate();
  expect(fast.isCurrent()).toBe(false);
  expect(requests.begin().isCurrent()).toBe(true);
});
