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

$ENV{CSVDB_FFI_LIB} = $lib;
require Csvdb;

plan tests => 7;

# Test version
my $version = Csvdb::version();
ok($version =~ /^0\./, "version starts with 0.: $version");

# Test checksum on a temp .csvdb directory
my $tmpdir = tempdir(CLEANUP => 1);
my $csvdb_dir = File::Spec->catdir($tmpdir, "test.csvdb");
mkdir $csvdb_dir;

# Write schema.sql
open my $fh, '>', File::Spec->catfile($csvdb_dir, 'schema.sql') or die $!;
print $fh qq{CREATE TABLE "users" (\n    "id" INTEGER PRIMARY KEY,\n    "name" TEXT NOT NULL\n);\n};
close $fh;

# Write users.csv
open $fh, '>', File::Spec->catfile($csvdb_dir, 'users.csv') or die $!;
print $fh "id,name\n1,Alice\n2,Bob\n";
close $fh;

# Test checksum
my $hash = Csvdb::checksum(input => $csvdb_dir);
is(length($hash), 64, "checksum returns 64-char hex string");
ok($hash =~ /^[0-9a-f]+$/, "checksum is hex");

# Test validate
my $rc = Csvdb::validate(input => $csvdb_dir);
is($rc, 0, "validate returns 0 for valid directory");

# Test diff (same dir vs itself)
my $diff_rc = Csvdb::diff(left => $csvdb_dir, right => $csvdb_dir);
is($diff_rc, 0, "diff of identical dirs returns 0");

# Test SQL query
my $csv_result = Csvdb::sql(path => $csvdb_dir, query => "SELECT name FROM users ORDER BY id");
like($csv_result, qr/Alice/, "SQL result contains Alice");
like($csv_result, qr/Bob/, "SQL result contains Bob");
