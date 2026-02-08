/**
 * Base error class for the BLS package.
 * Equivalent to RelayError in the relay package.
 */
export class BlsBaseError extends Error {
  constructor(
    message: string,
    public code: string
  ) {
    super(message);
    this.name = 'BlsBaseError';
  }
}

/**
 * Error thrown when environment variable configuration is invalid.
 * Includes the variable name and specific validation failure reason.
 */
export class ConfigError extends BlsBaseError {
  constructor(
    public readonly variable: string,
    message: string
  ) {
    super(`${variable}: ${message}`, 'CONFIG_ERROR');
    this.name = 'ConfigError';
  }
}
