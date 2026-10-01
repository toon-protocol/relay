import { describe, expect } from 'vitest';
import { conformanceTest, implementationName } from './implementation.js';

// The expected-failure marker, proved on itself: these need no relay.
describe('conformanceTest: expected failures', () => {
  conformanceTest(
    'a test marked for the implementation under test runs as it.fails',
    () => {
      expect('passes').toBe('fails');
    },
    { expectedFailureFor: [implementationName()] }
  );

  conformanceTest(
    'a test marked for another implementation runs as a plain test',
    () => {
      expect('passes').toBe('passes');
    },
    { expectedFailureFor: [`not-${implementationName()}`] }
  );
});
