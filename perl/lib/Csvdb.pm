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

$ffi->attach(csvdb_to_csvdb    => ['string','string','string','string','int'] => 'opaque');
$ffi->attach(csvdb_to_sqlite   => ['string','string','int']       => 'opaque');
$ffi->attach(csvdb_to_duckdb   => ['string','string','int']       => 'opaque');
$ffi->attach(csvdb_to_parquetdb=> ['string','string','string','string','int'] => 'opaque');
$ffi->attach(csvdb_checksum    => ['string']                      => 'opaque');
$ffi->attach(csvdb_diff        => ['string','string','int','string','string'] => 'int');
$ffi->attach(csvdb_validate    => ['string']                      => 'int');
$ffi->attach(csvdb_sql         => ['string','string']             => 'opaque');
$ffi->attach(csvdb_init        => ['string','string','int','string','string'] => 'opaque');

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
    my $input     = $args{input}     // croak "input is required";
    my $output    = $args{output}    // undef;
    my $order     = $args{order}     // undef;
    my $null_mode = $args{null_mode} // undef;
    my $force     = $args{force}     ? 1 : 0;
    return _read_string(csvdb_to_csvdb($input, $output, $order, $null_mode, $force));
}

sub to_sqlite {
    my (%args) = @_;
    my $input  = $args{input}  // croak "input is required";
    my $output = $args{output} // undef;
    my $force  = $args{force}  ? 1 : 0;
    return _read_string(csvdb_to_sqlite($input, $output, $force));
}

sub to_duckdb {
    my (%args) = @_;
    my $input  = $args{input}  // croak "input is required";
    my $output = $args{output} // undef;
    my $force  = $args{force}  ? 1 : 0;
    return _read_string(csvdb_to_duckdb($input, $output, $force));
}

sub to_parquetdb {
    my (%args) = @_;
    my $input     = $args{input}     // croak "input is required";
    my $output    = $args{output}    // undef;
    my $order     = $args{order}     // undef;
    my $null_mode = $args{null_mode} // undef;
    my $force     = $args{force}     ? 1 : 0;
    return _read_string(csvdb_to_parquetdb($input, $output, $order, $null_mode, $force));
}

sub checksum {
    my (%args) = @_;
    my $input = $args{input} // croak "input is required";
    return _read_string(csvdb_checksum($input));
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
    my $source  = $args{source}  // croak "source is required";
    my $output  = $args{output}  // undef;
    my $force   = $args{force}   ? 1 : 0;
    my $tables  = $args{tables}  // undef;
    my $exclude = $args{exclude} // undef;
    return _read_string(csvdb_init($source, $output, $force, $tables, $exclude));
}

1;

__END__

=head1 NAME

Csvdb - Perl bindings for csvdb via C FFI

=head1 SYNOPSIS

    use Csvdb;

    print Csvdb::version(), "\n";

    my $path = Csvdb::to_sqlite(input => "data.csvdb");
    my $hash = Csvdb::checksum(input => "data.csvdb");
    my $csv  = Csvdb::sql(path => "data.sqlite", query => "SELECT * FROM users");

=head1 DESCRIPTION

Perl bindings for csvdb using the C FFI shared library (libcsvdb_ffi).

Set the C<CSVDB_FFI_LIB> environment variable to the path of the shared library,
or ensure it is in a standard location.

=cut
