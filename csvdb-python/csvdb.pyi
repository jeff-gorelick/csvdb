"""Type stubs for the csvdb native module."""

from typing import Any, Dict, List, Optional, Union, overload

import pyarrow as pa

@overload
def to_csvdb(
    input: Dict[str, Any],
    *,
    output: str,
    force: bool = False,
    order: str = "pk",
    null_mode: str = "marker",
    natural_sort: bool = False,
    order_by: Optional[str] = None,
    compress: bool = False,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> str: ...
@overload
def to_csvdb(
    input: str,
    *,
    output: Optional[str] = None,
    order: str = "pk",
    null_mode: str = "marker",
    natural_sort: bool = False,
    order_by: Optional[str] = None,
    compress: bool = False,
    force: bool = False,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> str: ...
def to_csvdb_incremental(
    input: str,
    *,
    output: Optional[str] = None,
    order: str = "pk",
    null_mode: str = "marker",
    natural_sort: bool = False,
    order_by: Optional[str] = None,
    compress: bool = False,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> Dict[str, Any]: ...
def to_sqlite(
    input: str,
    *,
    output: Optional[str] = None,
    force: bool = False,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> str: ...
def to_duckdb(
    input: str,
    *,
    output: Optional[str] = None,
    force: bool = False,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> str: ...
def to_parquetdb(
    input: str,
    *,
    output: Optional[str] = None,
    order: str = "pk",
    null_mode: str = "marker",
    order_by: Optional[str] = None,
    force: bool = False,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> str: ...
def sql(path: str, query: str) -> List[Dict[str, Any]]: ...
def checksum(
    path: str,
    *,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> str: ...
def diff(
    left: str,
    right: str,
    *,
    summary: bool = False,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> bool: ...
def validate(path: str) -> Dict[str, Any]: ...
def init(
    source: str,
    *,
    output: Optional[str] = None,
    force: bool = False,
    detect_pk: bool = True,
    detect_fk: bool = True,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> Dict[str, Any]: ...
@overload
def to_arrow(
    path: str,
    table: str,
    *,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> pa.Table: ...
@overload
def to_arrow(
    path: str,
    table: None = None,
    *,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> Dict[str, pa.Table]: ...
def sql_arrow(path: str, query: str) -> pa.Table: ...

# The following functions require optional dependencies.
# Install with: pip install csvdb-py[pandas] or csvdb-py[polars]

@overload
def to_pandas(
    path: str,
    table: str,
    *,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> "Any": ...  # pd.DataFrame
@overload
def to_pandas(
    path: str,
    table: None = None,
    *,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> "Dict[str, Any]": ...  # Dict[str, pd.DataFrame]
def sql_pandas(path: str, query: str) -> "Any": ...  # pd.DataFrame
@overload
def to_polars(
    path: str,
    table: str,
    *,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> "Any": ...  # pl.DataFrame
@overload
def to_polars(
    path: str,
    table: None = None,
    *,
    tables: List[str] = [],
    exclude: List[str] = [],
) -> "Dict[str, Any]": ...  # Dict[str, pl.DataFrame]
def sql_polars(path: str, query: str) -> "Any": ...  # pl.DataFrame
def version() -> str: ...
