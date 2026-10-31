#!/bin/sh
set -eu
APP_PASSWORD=$(cat /run/colossus/app-password)
export APP_PASSWORD
psql -X -q -v ON_ERROR_STOP=1 --username postgres --dbname postgres <<'SQL'
\getenv app_password APP_PASSWORD
SELECT format('CREATE ROLE colossus LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD %L', :'app_password') \gexec
CREATE DATABASE colossus OWNER colossus;
REVOKE ALL ON DATABASE colossus FROM PUBLIC;
GRANT CONNECT,TEMPORARY ON DATABASE colossus TO colossus;
\connect colossus
REVOKE ALL ON SCHEMA public FROM PUBLIC;
SQL
unset APP_PASSWORD
