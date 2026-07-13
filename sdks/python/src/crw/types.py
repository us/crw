"""Typed native extract lifecycle contracts."""

from typing import Any, Literal, TypedDict

ExtractJobState = Literal["processing", "cancelling", "completed", "failed", "cancelled"]
ExtractUrlState = Literal["processing", "completed", "failed", "cancelled"]
FieldStatus = Literal["supported", "unverified", "unsupported", "notFound"]


class _EvidenceCitationRequired(TypedDict):
    url: str
    sourceHash: str
    sourceTextKind: str


class EvidenceCitation(_EvidenceCitationRequired, total=False):
    title: str
    excerpt: str


class _BasisRequired(TypedDict):
    basisVersion: int
    field: str
    value: Any | None
    status: FieldStatus
    citations: list[EvidenceCitation]


class Basis(_BasisRequired, total=False):
    confidence: Literal["low", "medium", "high"]


class BasisWarning(TypedDict):
    field: str
    code: str


class ExtractAccepted(TypedDict):
    id: str
    status: Literal["processing"]
    urls: int


class _ExtractUrlResultRequired(TypedDict):
    url: str
    status: ExtractUrlState


class ExtractUrlResult(_ExtractUrlResultRequired, total=False):
    data: Any
    error: str
    llmUsage: dict[str, Any]
    basis: list[Basis]
    basisWarnings: list[BasisWarning]
    llmInputHash: str


class _ExtractStatusRequired(TypedDict):
    id: str
    status: ExtractJobState
    results: list[ExtractUrlResult]
    expiresAt: str
    creditsUsed: int
    tokensUsed: int


class ExtractStatus(_ExtractStatusRequired, total=False):
    error: str
