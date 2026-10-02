import { it } from 'vitest';

/** The image reference under test. */
export function imageUnderTest(): string {
  const image = process.env['CONFORMANCE_IMAGE'];
  if (image === undefined || image === '') {
    throw new Error(
      'CONFORMANCE_IMAGE is not set: name the relay image to test, e.g. ' +
        'CONFORMANCE_IMAGE=relay:ci'
    );
  }
  return image;
}

/** The name of the implementation under test (default `rust`). */
export function implementationName(): string {
  return process.env['CONFORMANCE_IMPL'] || 'rust';
}

export interface ConformanceTestOptions {
  /** Implementations known to fail this test. */
  expectedFailureFor?: readonly string[];
}

/**
 * Declare a test. Under an implementation listed in `expectedFailureFor` it
 * is an expected failure (`it.fails`): it must still fail, and turns red the
 * day it passes so the marker cannot rot. Under any other it is plain `it`.
 */
export function conformanceTest(
  name: string,
  fn: () => Promise<void> | void,
  options: ConformanceTestOptions = {}
): void {
  const expected = options.expectedFailureFor?.includes(implementationName());
  if (expected) {
    it.fails(`${name} [expected failure: ${implementationName()}]`, fn);
  } else {
    it(name, fn);
  }
}

/** `conformanceTest` once per case, named by `name(case)`. */
export function conformanceTestEach<T>(
  cases: readonly T[],
  name: (testCase: T) => string,
  fn: (testCase: T) => Promise<void> | void,
  options: ConformanceTestOptions = {}
): void {
  for (const testCase of cases) {
    conformanceTest(name(testCase), () => fn(testCase), options);
  }
}
