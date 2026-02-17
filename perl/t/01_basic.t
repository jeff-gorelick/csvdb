#!/usr/bin/env perl
use strict;
use warnings;
use Test::More;
use FindBin;
use File::Temp qw(tempdir);
use File::Spec;
use lib File::Spec->catdir($FindBin::Bin, '..', 'lib');

# Check FFI::Platypus is available
eval { require FFI::Platypus };
if ($@) {
    plan skip_all => "FFI::Platypus not installed";
}

# Find the shared library
my $repo_root = File::Spec->catdir($FindBin::Bin, '..', '..');
my $lib;
for my $candidate (
    File::Spec->catfile($repo_root, 'target', 'release', 'libcsvdb_ffi.dylib'),
    File::Spec->catfile($repo_root, 'target', 'release', 'libcsvdb_ffi.so'),
    File::Spec->catfile($repo_root, 'target', 'release', 'csvdb_ffi.dll'),
    File::Spec->catfile($repo_root, 'target', 'debug', 'libcsvdb_ffi.dylib'),
    File::Spec->catfile($repo_root, 'target', 'debug', 'libcsvdb_ffi.so'),
    File::Spec->catfile($repo_root, 'target', 'debug', 'csvdb_ffi.dll'),
) {
    if (-f $candidate) {
        $lib = $candidate;
        last;
    }
}

unless ($lib) {
    plan skip_all => "libcsvdb_ffi not found. Build with: cargo build --release -p csvdb-ffi";
}

local $ENV{CSVDB_FFI_LIB} = $lib;
require Csvdb;

plan tests => 25;

# Helper: create a sample .csvdb directory
sub make_csvdb {
    my ($dir) = @_;
    my $csvdb_dir = File::Spec->catdir($dir, "test.csvdb");
    mkdir $csvdb_dir;

    open my $sfh, '>', File::Spec->catfile($csvdb_dir, 'schema.sql') or die $!;
    print $sfh qq{CREATE TABLE "users" (\n    "id" INTEGER PRIMARY KEY,\n    "name" TEXT NOT NULL\n);\n};
    close $sfh;

    open my $cfh, '>', File::Spec->catfile($csvdb_dir, 'users.csv') or die $!;
    print $cfh "id,name\n1,Alice\n2,Bob\n";
    close $cfh;

    return $csvdb_dir;
}

# Helper: create a SQLite database
sub make_sqlite {
    my ($dir) = @_;
    my $csvdb_dir = make_csvdb($dir);
    my $sqlite_path = Csvdb::to_sqlite(input => $csvdb_dir, force => 1);
    return $sqlite_path;
}

# Helper: create a multi-table .csvdb directory
sub make_multi_csvdb {
    my ($dir) = @_;
    my $csvdb_dir = File::Spec->catdir($dir, "multi.csvdb");
    mkdir $csvdb_dir;

    open my $sfh, '>', File::Spec->catfile($csvdb_dir, 'schema.sql') or die $!;
    print $sfh qq{CREATE TABLE "users" (\n    "id" INTEGER PRIMARY KEY,\n    "name" TEXT NOT NULL\n);\n};
    print $sfh qq{CREATE TABLE "orders" (\n    "id" INTEGER PRIMARY KEY,\n    "user_id" INTEGER,\n    "amount" REAL\n);\n};
    close $sfh;

    open my $ufh, '>', File::Spec->catfile($csvdb_dir, 'users.csv') or die $!;
    print $ufh "id,name\n1,Alice\n2,Bob\n";
    close $ufh;

    open my $ofh, '>', File::Spec->catfile($csvdb_dir, 'orders.csv') or die $!;
    print $ofh "id,user_id,amount\n100,1,99.99\n101,2,49.50\n";
    close $ofh;

    return $csvdb_dir;
}

# Helper: create a directory with raw CSV files
sub make_raw_csv {
    my ($dir) = @_;
    my $csv_dir = File::Spec->catdir($dir, "raw");
    mkdir $csv_dir;

    open my $fh, '>', File::Spec->catfile($csv_dir, 'products.csv') or die $!;
    print $fh "id,name,price\n1,Widget,9.99\n2,Gadget,19.99\n";
    close $fh;

    return $csv_dir;
}

my $tmpdir = tempdir(CLEANUP => 1);

# --- Test version ---
my $version = Csvdb::version();
ok($version =~ /^0\./, "version starts with 0.: $version");

# --- Test checksum ---
my $csvdb_dir = make_csvdb($tmpdir);

my $hash = Csvdb::checksum(input => $csvdb_dir);
is(length($hash), 64, "checksum returns 64-char hex string");
ok($hash =~ /^[0-9a-f]+$/, "checksum is hex");

# checksum is deterministic
my $hash2 = Csvdb::checksum(input => $csvdb_dir);
is($hash, $hash2, "checksum is deterministic");

# --- Test validate ---
my $rc = Csvdb::validate(input => $csvdb_dir);
is($rc, 0, "validate returns 0 for valid directory");

# --- Test diff ---
my $diff_rc = Csvdb::diff(left => $csvdb_dir, right => $csvdb_dir);
is($diff_rc, 0, "diff of identical dirs returns 0");

# --- Test SQL query ---
my $csv_result = Csvdb::sql(path => $csvdb_dir, query => "SELECT name FROM users ORDER BY id");
like($csv_result, qr/Alice/, "SQL result contains Alice");
like($csv_result, qr/Bob/, "SQL result contains Bob");

# --- Test to_sqlite ---
my $sqlite_path = Csvdb::to_sqlite(input => $csvdb_dir, force => 1);
ok(-f $sqlite_path, "to_sqlite creates a file");
like($sqlite_path, qr/\.sqlite$/, "to_sqlite output has .sqlite extension");

# checksum consistency across formats
my $sqlite_hash = Csvdb::checksum(input => $sqlite_path);
is($hash, $sqlite_hash, "checksum is consistent across csvdb and sqlite");

# --- Test to_csvdb (round-trip from sqlite) ---
my $rt_dir = File::Spec->catdir($tmpdir, "roundtrip.csvdb");
my $rt_path = Csvdb::to_csvdb(input => $sqlite_path, output => $rt_dir, force => 1);
ok(-d $rt_path, "to_csvdb creates a directory");
ok(-f File::Spec->catfile($rt_path, 'schema.sql'), "to_csvdb output has schema.sql");
ok(-f File::Spec->catfile($rt_path, 'users.csv'), "to_csvdb output has users.csv");

# --- Test to_parquetdb ---
my $pqdb_dir = File::Spec->catdir($tmpdir, "test.parquetdb");
my $pqdb_path = Csvdb::to_parquetdb(input => $csvdb_dir, output => $pqdb_dir, force => 1);
ok(-d $pqdb_path, "to_parquetdb creates a directory");

# --- Test init ---
my $raw_dir = make_raw_csv($tmpdir);
my $init_result = Csvdb::init(source => $raw_dir, force => 1);
ok(defined $init_result && length($init_result) > 0, "init returns output path");
ok(-d $init_result, "init creates output directory");

# --- Test checksum with tables filter ---
my $multi_dir = make_multi_csvdb($tmpdir);
my $full_hash = Csvdb::checksum(input => $multi_dir);
my $partial_hash = Csvdb::checksum(input => $multi_dir, tables => "users");
isnt($full_hash, $partial_hash, "checksum with tables filter differs from full checksum");

# --- Test diff with tables filter ---
my $diff_filtered = Csvdb::diff(left => $multi_dir, right => $multi_dir, tables => "users");
is($diff_filtered, 0, "diff with tables filter works on identical data");

# --- Test SQL on csvdb with WHERE ---
my $sql_where = Csvdb::sql(path => $csvdb_dir, query => "SELECT name FROM users WHERE id = 1");
like($sql_where, qr/Alice/, "SQL with WHERE returns Alice");
unlike($sql_where, qr/Bob/, "SQL with WHERE excludes Bob");

# --- Test to_csvdb with compress ---
my $comp_dir = File::Spec->catdir($tmpdir, "compressed.csvdb");
my $comp_path = Csvdb::to_csvdb(input => $sqlite_path, output => $comp_dir, force => 1, compress => 1);
ok(-f File::Spec->catfile($comp_path, 'users.csv.gz'), "to_csvdb compress creates .csv.gz");

# --- Test to_csvdb_incremental ---
my $inc_dir = File::Spec->catdir($tmpdir, "incremental.csvdb");
my $inc_json = Csvdb::to_csvdb_incremental(input => $sqlite_path, output => $inc_dir);
like($inc_json, qr/"path"/, "to_csvdb_incremental returns JSON with path");
like($inc_json, qr/"added"/, "to_csvdb_incremental returns JSON with added");

# --- Test init with detect_pk disabled ---
my $raw_dir2 = File::Spec->catdir($tmpdir, "raw2");
mkdir $raw_dir2;
open my $fh2, '>', File::Spec->catfile($raw_dir2, 'items.csv') or die $!;
print $fh2 "id,name\n1,Foo\n2,Bar\n";
close $fh2;
my $init_nopk = Csvdb::init(source => $raw_dir2, force => 1, detect_pk => 0);
ok(defined $init_nopk && length($init_nopk) > 0, "init with detect_pk=0 returns output path");
