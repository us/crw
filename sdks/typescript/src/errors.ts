/** CRW SDK error types. */

export class CrwError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CrwError";
  }
}

export class CrwApiError extends CrwError {
  statusCode?: number;
  constructor(message: string, statusCode?: number) {
    super(message);
    this.name = "CrwApiError";
    this.statusCode = statusCode;
  }
}

export class CrwTimeoutError extends CrwError {
  constructor(message: string) {
    super(message);
    this.name = "CrwTimeoutError";
  }
}

export class CrwBinaryNotFoundError extends CrwError {
  constructor(message: string) {
    super(message);
    this.name = "CrwBinaryNotFoundError";
  }
}
