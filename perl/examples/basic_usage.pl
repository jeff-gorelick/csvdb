#!/usr/bin/env perl
# Basic usage examples for csvdb Perl bindings.
#
# Setup:
#     cargo build --release -p csvdb-ffi
#     cpanm FFI::Platypus   (if not already installed)
#
# Run from repo root:
#     perl -Iperl/lib perl/examples/basic_usage.pl

use strict;
use warnings;
use File::Temp qw(tempdir);
use File::Spec;
use FindBin;
use lib File::Spec->catdir($FindBin::Bin, '..', 'lib');

# Point to the shared library
my $repo_root = File::Spec->catdir($FindBin::Bin, '..', '..');
for my $ext (qw(dylib so)) {
    my $path = File::Spec->catfile($repo_root, 'target', 'release', "libcsvdb_ffi.$ext");
    if (-f $path) {
        $ENV{CSVDB_FFI_LIB} = $path;
        last;
    }
}

require Csvdb;

print "csvdb version: ", Csvdb::version(), "\n\n";

# Create a sample .csvdb directory
my $tmpdir = tempdir(CLEANUP => 1);
my $csvdb_dir = File::Spec->catdir($tmpdir, "shop.csvdb");
mkdir $csvdb_dir;

# Write schema.sql
open my $fh, '>', File::Spec->catfile($csvdb_dir, 'schema.sql') or die $!;
print $fh <<'SQL';
CREATE TABLE "users" (
    "id" INTEGER PRIMARY KEY,
    "name" TEXT NOT NULL,
    "email" TEXT
);

CREATE TABLE "orders" (
    "id" INTEGER PRIMARY KEY,
    "user_id" INTEGER NOT NULL REFERENCES "users"("id"),
    "product" TEXT NOT NULL,
    "amount" REAL NOT NULL
);
SQL
close $fh;

# Write users.csv
open $fh, '>', File::Spec->catfile($csvdb_dir, 'users.csv') or die $!;
print $fh <<'CSV';
"id","name","email"
"1","Alice","alice@example.com"
"2","Bob","bob@example.com"
"3","Charlie",\N
CSV
close $fh;

# Write orders.csv
open $fh, '>', File::Spec->catfile($csvdb_dir, 'orders.csv') or die $!;
print $fh <<'CSV';
"id","user_id","product","amount"
"1","1","Widget","9.99"
"2","1","Gadget","24.99"
"3","2","Widget","9.99"
"4","3","Gizmo","49.99"
CSV
close $fh;

print "Created sample csvdb: $csvdb_dir\n";

# --- Checksum ---
my $hash = Csvdb::checksum(input => $csvdb_dir);
print "\nChecksum: $hash\n";

# --- Validate ---
my $rc = Csvdb::validate(input => $csvdb_dir);
print "\nValidation: ", ($rc == 0 ? "valid" : "errors found"), "\n";

# --- SQL query ---
my $csv = Csvdb::sql(path => $csvdb_dir, query => <<'SQL');
SELECT u.name, SUM(o.amount) AS total
FROM users u
JOIN orders o ON u.id = o.user_id
GROUP BY u.name
ORDER BY total DESC
SQL
print "\nSpending by customer (CSV):\n$csv\n";

# --- Diff (identical) ---
my $diff_rc = Csvdb::diff(left => $csvdb_dir, right => $csvdb_dir);
print "Diff against itself: ", ($diff_rc == 0 ? "identical" : "differences found"), "\n";

# --- Convert to SQLite ---
my $sqlite_out = Csvdb::to_sqlite(input => $csvdb_dir, force => 1);
print "\nConverted to SQLite: $sqlite_out\n";

# --- Convert to DuckDB ---
my $duckdb_out = Csvdb::to_duckdb(input => $csvdb_dir, force => 1);
print "Converted to DuckDB: $duckdb_out\n";

# --- Query the DuckDB file ---
my $count_csv = Csvdb::sql(path => $duckdb_out, query => "SELECT COUNT(*) AS n FROM orders");
print "\nOrders in DuckDB:\n$count_csv";

print "\nDone.\n";
