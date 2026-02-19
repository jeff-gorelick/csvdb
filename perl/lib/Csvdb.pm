package Csvdb;
use strict;
use warnings;
use FFI::Platypus 2.00;
use FFI::Platypus::Memory qw(strdup free);
use Carp qw(croak);

our $VERSION = '0.2.0';

my $ffi = FFI::Platypus->new(api => 2);

# Find the shared library - check common locations
my @lib_search = (
    $ENV{CSVDB_FFI_LIB},
    'libcsvdb_ffi.dylib',
    'libcsvdb_ffi.so',
    'csvdb_ffi.dll',
);

# Also search relative to this module's location
my $module_dir;
if (__FILE__ =~ m{^(.+)/lib/Csvdb\.pm$}) {
    $module_dir = $1;
    unshift @lib_search,
        "$module_dir/../target/release/libcsvdb_ffi.dylib",
        "$module_dir/../target/release/libcsvdb_ffi.so",
        "$module_dir/../target/debug/libcsvdb_ffi.dylib",
        "$module_dir/../target/debug/libcsvdb_ffi.so";
}

my $lib_path;
for my $candidate (@lib_search) {
    next unless defined $candidate;
    if (-f $candidate) {
        $lib_path = $candidate;
        last;
    }
}

croak "Cannot find libcsvdb_ffi shared library. Set CSVDB_FFI_LIB env var or build with: cargo build --release -p csvdb-ffi"
    unless $lib_path;

$ffi->lib($lib_path);

# Attach FFI functions
$ffi->attach(csvdb_version     => []                              => 'string');
$ffi->attach(csvdb_last_error  => []                              => 'string');
$ffi->attach(csvdb_free_string => ['opaque']                      => 'void');

$ffi->attach(csvdb_to_csvdb    => ['string','string','string','string','int','string','string','int','string','int'] => 'opaque');
$ffi->attach(csvdb_to_csvdb_incremental => ['string','string','string','string','string','string','int','string','int'] => 'opaque');
$ffi->attach(csvdb_to_sqlite   => ['string','string','int','string','string']       => 'opaque');
$ffi->attach(csvdb_to_duckdb   => ['string','string','int','string','string']       => 'opaque');
$ffi->attach(csvdb_to_parquetdb=> ['string','string','string','string','int','string','string','string'] => 'opaque');
$ffi->attach(csvdb_checksum    => ['string','string','string']    => 'opaque');
$ffi->attach(csvdb_diff        => ['string','string','int','string','string'] => 'int');
$ffi->attach(csvdb_diff_json   => ['string','string','int','string','string'] => 'opaque');
$ffi->attach(csvdb_validate    => ['string']                      => 'int');
$ffi->attach(csvdb_sql         => ['string','string']             => 'opaque');
$ffi->attach(csvdb_init        => ['string','string','int','string','string','int','int'] => 'opaque');

# Helper: read and free a returned string, or die with last error
sub _read_string {
    my ($ptr) = @_;
    if (!$ptr) {
        my $err = csvdb_last_error() // 'unknown error';
        croak "csvdb error: $err";
    }
    my $str = $ffi->cast('opaque' => 'string', $ptr);
    csvdb_free_string($ptr);
    return $str;
}

# Public API

sub version {
    return csvdb_version();
}

sub to_csvdb {
    my (%args) = @_;
    my $input        = $args{input}        // croak "input is required";
    my $output       = $args{output}       // undef;
    my $order        = $args{order}        // undef;
    my $null_mode    = $args{null_mode}    // undef;
    my $force        = $args{force}        ? 1 : 0;
    my $tables       = $args{tables}       // undef;
    my $exclude      = $args{exclude}      // undef;
    my $natural_sort = $args{natural_sort} ? 1 : 0;
    my $order_by     = $args{order_by}     // undef;
    my $compress     = $args{compress}     ? 1 : 0;
    return _read_string(csvdb_to_csvdb($input, $output, $order, $null_mode, $force, $tables, $exclude, $natural_sort, $order_by, $compress));
}

sub to_csvdb_incremental {
    my (%args) = @_;
    my $input        = $args{input}        // croak "input is required";
    my $output       = $args{output}       // undef;
    my $order        = $args{order}        // undef;
    my $null_mode    = $args{null_mode}    // undef;
    my $tables       = $args{tables}       // undef;
    my $exclude      = $args{exclude}      // undef;
    my $natural_sort = $args{natural_sort} ? 1 : 0;
    my $order_by     = $args{order_by}     // undef;
    my $compress     = $args{compress}     ? 1 : 0;
    return _read_string(csvdb_to_csvdb_incremental($input, $output, $order, $null_mode, $tables, $exclude, $natural_sort, $order_by, $compress));
}

sub to_sqlite {
    my (%args) = @_;
    my $input   = $args{input}   // croak "input is required";
    my $output  = $args{output}  // undef;
    my $force   = $args{force}   ? 1 : 0;
    my $tables  = $args{tables}  // undef;
    my $exclude = $args{exclude} // undef;
    return _read_string(csvdb_to_sqlite($input, $output, $force, $tables, $exclude));
}

sub to_duckdb {
    my (%args) = @_;
    my $input   = $args{input}   // croak "input is required";
    my $output  = $args{output}  // undef;
    my $force   = $args{force}   ? 1 : 0;
    my $tables  = $args{tables}  // undef;
    my $exclude = $args{exclude} // undef;
    return _read_string(csvdb_to_duckdb($input, $output, $force, $tables, $exclude));
}

sub to_parquetdb {
    my (%args) = @_;
    my $input     = $args{input}     // croak "input is required";
    my $output    = $args{output}    // undef;
    my $order     = $args{order}     // undef;
    my $null_mode = $args{null_mode} // undef;
    my $force     = $args{force}     ? 1 : 0;
    my $tables    = $args{tables}    // undef;
    my $exclude   = $args{exclude}   // undef;
    my $order_by  = $args{order_by}  // undef;
    return _read_string(csvdb_to_parquetdb($input, $output, $order, $null_mode, $force, $tables, $exclude, $order_by));
}

sub checksum {
    my (%args) = @_;
    my $input   = $args{input}   // croak "input is required";
    my $tables  = $args{tables}  // undef;
    my $exclude = $args{exclude} // undef;
    return _read_string(csvdb_checksum($input, $tables, $exclude));
}

sub diff {
    my (%args) = @_;
    my $left    = $args{left}    // croak "left is required";
    my $right   = $args{right}   // croak "right is required";
    my $summary = $args{summary} ? 1 : 0;
    my $tables  = $args{tables}  // undef;
    my $exclude = $args{exclude} // undef;
    my $rc = csvdb_diff($left, $right, $summary, $tables, $exclude);
    if ($rc == -1) {
        my $err = csvdb_last_error() // 'unknown error';
        croak "csvdb error: $err";
    }
    return $rc;
}

sub diff_json {
    my (%args) = @_;
    my $left    = $args{left}    // croak "left is required";
    my $right   = $args{right}   // croak "right is required";
    my $summary = $args{summary} ? 1 : 0;
    my $tables  = $args{tables}  // undef;
    my $exclude = $args{exclude} // undef;
    return _read_string(csvdb_diff_json($left, $right, $summary, $tables, $exclude));
}

sub validate {
    my (%args) = @_;
    my $input = $args{input} // croak "input is required";
    my $rc = csvdb_validate($input);
    if ($rc == -1) {
        my $err = csvdb_last_error() // 'unknown error';
        croak "csvdb error: $err";
    }
    return $rc;
}

sub sql {
    my (%args) = @_;
    my $path  = $args{path}  // croak "path is required";
    my $query = $args{query} // croak "query is required";
    return _read_string(csvdb_sql($path, $query));
}

sub init {
    my (%args) = @_;
    my $source    = $args{source}    // croak "source is required";
    my $output    = $args{output}    // undef;
    my $force     = $args{force}     ? 1 : 0;
    my $tables    = $args{tables}    // undef;
    my $exclude   = $args{exclude}   // undef;
    my $detect_pk = exists $args{detect_pk} ? ($args{detect_pk} ? 1 : 0) : 1;
    my $detect_fk = exists $args{detect_fk} ? ($args{detect_fk} ? 1 : 0) : 1;
    return _read_string(csvdb_init($source, $output, $force, $tables, $exclude, $detect_pk, $detect_fk));
}

1;

__END__

=head1 NAME

Csvdb - Perl bindings for csvdb via C FFI

=head1 SYNOPSIS

    use Csvdb;

    print Csvdb::version(), "\n";

    # Convert between formats
    my $csvdb_path = Csvdb::to_csvdb(input => "data.sqlite", force => 1);
    my $sqlite_path = Csvdb::to_sqlite(input => "data.csvdb", force => 1);
    my $duckdb_path = Csvdb::to_duckdb(input => "data.csvdb", force => 1);
    my $pqdb_path = Csvdb::to_parquetdb(input => "data.csvdb", output => "out.parquetdb");

    # Incremental export (only re-exports changed tables)
    my $json = Csvdb::to_csvdb_incremental(input => "data.sqlite", output => "out.csvdb");

    # Query, checksum, diff, validate
    my $csv = Csvdb::sql(path => "data.csvdb", query => "SELECT * FROM users");
    my $hash = Csvdb::checksum(input => "data.csvdb");
    my $has_diff = Csvdb::diff(left => "a.csvdb", right => "b.csvdb");
    my $valid = Csvdb::validate(input => "data.csvdb");

    # Initialize from raw CSV files
    my $path = Csvdb::init(source => "/path/to/csv/dir");

=head1 DESCRIPTION

Perl bindings for csvdb using the C FFI shared library (libcsvdb_ffi).
Requires L<FFI::Platypus> 2.00 or later.

All functions die on error with a descriptive message prefixed by C<csvdb error:>.

=head1 FUNCTIONS

=head2 version

    my $v = Csvdb::version();

Returns the csvdb library version string.

=head2 to_csvdb

    my $path = Csvdb::to_csvdb(
        input        => $input_path,   # required
        output       => $output_dir,   # optional
        order        => "pk",          # "pk", "all-columns", or "add-synthetic-key"
        null_mode    => "marker",      # "marker", "empty", or "literal"
        force        => 0,             # overwrite existing output
        tables       => "t1,t2",       # comma-separated table filter
        exclude      => "logs",        # comma-separated tables to exclude
        natural_sort => 0,             # sort string PKs naturally
        order_by     => "col DESC",    # custom ORDER BY clause
        compress     => 0,             # gzip compress CSV files
    );

Converts any supported format (.sqlite, .duckdb, .parquetdb) to a .csvdb directory.
Returns the output directory path.

=head2 to_csvdb_incremental

    my $json = Csvdb::to_csvdb_incremental(
        input        => $input_path,   # required
        output       => $output_dir,   # optional
        order        => "pk",
        null_mode    => "marker",
        tables       => undef,
        exclude      => undef,
        natural_sort => 0,
        order_by     => undef,
        compress     => 0,
    );

Incremental export: only re-exports tables whose data has changed.
Returns a JSON string with C<path>, C<unchanged>, C<updated>, C<added>, and C<removed> fields.

=head2 to_sqlite

    my $path = Csvdb::to_sqlite(
        input   => $input_path,   # required
        output  => $output_path,  # optional
        force   => 0,
        tables  => undef,
        exclude => undef,
    );

Converts to a SQLite database. Returns the output file path.

=head2 to_duckdb

    my $path = Csvdb::to_duckdb(
        input   => $input_path,   # required
        output  => $output_path,  # optional
        force   => 0,
        tables  => undef,
        exclude => undef,
    );

Converts to a DuckDB database. Returns the output file path.

=head2 to_parquetdb

    my $path = Csvdb::to_parquetdb(
        input     => $input_path,   # required
        output    => $output_dir,   # optional
        order     => "pk",
        null_mode => "marker",
        force     => 0,
        tables    => undef,
        exclude   => undef,
        order_by  => undef,
    );

Converts to a .parquetdb directory. Returns the output directory path.

=head2 checksum

    my $hash = Csvdb::checksum(
        input   => $path,     # required
        tables  => undef,
        exclude => undef,
    );

Computes a SHA-256 checksum of the database content. Returns a 64-character hex string.
The checksum is consistent across formats (the same data in .csvdb and .sqlite produces
the same hash).

=head2 diff

    my $rc = Csvdb::diff(
        left    => $left_path,    # required
        right   => $right_path,   # required
        summary => 0,
        tables  => undef,
        exclude => undef,
    );

Compares two databases. Returns 0 if identical, 1 if differences found.

=head2 diff_json

    my $json = Csvdb::diff_json(
        left    => $left_path,    # required
        right   => $right_path,   # required
        summary => 0,
        tables  => undef,
        exclude => undef,
    );

Compares two databases and returns the result as a JSON string with detailed
diff information including table-level and row-level changes.

=head2 validate

    my $rc = Csvdb::validate(input => $path);

Validates a .csvdb or .parquetdb directory. Returns 0 if valid, 1 if errors found.

=head2 sql

    my $csv = Csvdb::sql(
        path  => $db_path,   # required
        query => $sql,       # required (SELECT only)
    );

Runs a read-only SQL query against any supported format. Returns results as CSV text.

=head2 init

    my $path = Csvdb::init(
        source    => $csv_dir,   # required
        output    => undef,
        force     => 0,
        tables    => undef,
        exclude   => undef,
        detect_pk => 1,          # auto-detect primary keys
        detect_fk => 1,          # auto-detect foreign keys
    );

Initializes a .csvdb directory from raw CSV files with schema inference.
Returns the output directory path.

=head1 ENVIRONMENT

=over 4

=item C<CSVDB_FFI_LIB>

Path to the libcsvdb_ffi shared library. If not set, the module searches
standard locations and paths relative to its installation directory.

=back

=head1 SEE ALSO

L<https://github.com/jeff-gorelick/csvdb>

=cut
