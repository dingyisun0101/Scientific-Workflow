"""Official verified Python reader for Scientific Workflow recordings."""

from .errors import (
    DecoderError,
    IntegrityError,
    MetadataError,
    RecordError,
    RecordingError,
    RecordingNotCompleteError,
    UnknownStreamError,
)
from .state import StateField, StateRecord, StateSeries
from .reader import (
    FORMAT_NAME,
    FORMAT_VERSION,
    Decoder,
    RecordingReader,
    open_completed_recording,
)

__all__ = [
    "Decoder",
    "DecoderError",
    "FORMAT_NAME",
    "FORMAT_VERSION",
    "IntegrityError",
    "MetadataError",
    "RecordError",
    "RecordingError",
    "RecordingNotCompleteError",
    "RecordingReader",
    "StateField",
    "StateRecord",
    "StateSeries",
    "UnknownStreamError",
    "open_completed_recording",
]

__version__ = "0.4.3"
