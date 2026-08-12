"""Public exception hierarchy for recording validation and reconstruction."""


class RecordingError(Exception):
    """Base class for every reader failure."""


class MetadataError(RecordingError):
    """The authoritative metadata document violates format v4."""


class RecordingNotCompleteError(MetadataError):
    """The recording has not reached successful completion."""


class UnknownStreamError(RecordingError, KeyError):
    """The requested logical stream is not declared."""


class IntegrityError(RecordingError):
    """A declared immutable chunk is missing or fails integrity validation."""


class RecordError(RecordingError):
    """A JSONL state record violates its stream contract."""


class DecoderError(RecordingError):
    """A caller-supplied field decoder failed or is missing."""
