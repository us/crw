"""CRW SDK exceptions."""


class CrwError(Exception):
    """Base exception for CRW SDK."""


class CrwBinaryNotFoundError(CrwError):
    """Binary could not be found or downloaded."""


class CrwTimeoutError(CrwError):
    """Operation timed out."""


class CrwApiError(CrwError):
    """API returned an error."""

    def __init__(self, message: str, status_code: int | None = None):
        super().__init__(message)
        self.status_code = status_code
