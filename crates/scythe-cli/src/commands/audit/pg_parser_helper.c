#include <pg_query.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef SCYTHE_LIBPG_QUERY_RELEASE
#error SCYTHE_LIBPG_QUERY_RELEASE must name the pinned source release
#endif

#define MAX_SQL_BYTES (64u * 1024u * 1024u)

static char *read_sql(void) {
    size_t capacity = 4096;
    size_t length = 0;
    char *sql = malloc(capacity);
    if (sql == NULL) {
        return NULL;
    }

    for (;;) {
        if (length == capacity - 1) {
            if (capacity == MAX_SQL_BYTES + 1u) {
                fputs("parser helper: SQL input exceeds 64 MiB\n", stderr);
                free(sql);
                return NULL;
            }
            size_t next_capacity = capacity * 2;
            if (next_capacity > MAX_SQL_BYTES + 1u) {
                next_capacity = MAX_SQL_BYTES + 1u;
            }
            char *next = realloc(sql, next_capacity);
            if (next == NULL) {
                free(sql);
                return NULL;
            }
            sql = next;
            capacity = next_capacity;
        }

        size_t count = fread(sql + length, 1, capacity - length - 1, stdin);
        length += count;
        if (count == 0) {
            if (ferror(stdin)) {
                fputs("parser helper: failed to read SQL input\n", stderr);
                free(sql);
                return NULL;
            }
            break;
        }
    }
    sql[length] = '\0';
    if (memchr(sql, '\0', length) != NULL) {
        fputs("parser helper: SQL input contains a NUL byte\n", stderr);
        free(sql);
        return NULL;
    }
    return sql;
}

static int write_ast(const char *ast) {
    if (ast == NULL) {
        fputs("parser helper: parser returned no AST\n", stderr);
        return 1;
    }
    if (printf("{\"protocol\":1,\"pg_major\":%s,\"pg_version\":\"%s\",\"parser_release\":\"%s\",\"ast\":",
               PG_MAJORVERSION, PG_VERSION, SCYTHE_LIBPG_QUERY_RELEASE) < 0 ||
        fputs(ast, stdout) == EOF || fputs("}\n", stdout) == EOF || fflush(stdout) == EOF) {
        fputs("parser helper: failed to write AST\n", stderr);
        return 1;
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--version") == 0) {
        printf("scythe-pg%s-parser protocol/1 libpg_query/%s PostgreSQL/%s\n",
               PG_MAJORVERSION, SCYTHE_LIBPG_QUERY_RELEASE, PG_VERSION);
        return fflush(stdout) == EOF ? 1 : 0;
    }
    if (argc > 2 || (argc == 2 && strcmp(argv[1], "--plpgsql") != 0)) {
        fputs("usage: scythe-pgN-parser [--version|--plpgsql]\n", stderr);
        return 1;
    }

    char *sql = read_sql();
    if (sql == NULL) {
        fputs("parser helper: unable to buffer SQL input\n", stderr);
        return 1;
    }

    PgQuerySplitResult split = pg_query_split_with_parser(sql);
    if (split.error != NULL || split.n_stmts <= 0) {
        if (split.error != NULL) {
            const char *message = split.error->message != NULL ? split.error->message : "unknown parser error";
            fprintf(stderr, "parser helper: statement split failed: %s\n", message);
        } else {
            fputs("parser helper: SQL input contains zero statements\n", stderr);
        }
        pg_query_free_split_result(split);
        free(sql);
        pg_query_exit();
        return 1;
    }
    pg_query_free_split_result(split);

    int status;
    if (argc == 2) {
        PgQueryPlpgsqlParseResult result = pg_query_parse_plpgsql(sql);
        if (result.error != NULL) {
            const char *message = result.error->message != NULL ? result.error->message : "unknown parser error";
            fprintf(stderr, "parser helper: PL/pgSQL parse failed: %s\n", message);
            status = 1;
        } else {
            status = write_ast(result.plpgsql_funcs);
        }
        pg_query_free_plpgsql_parse_result(result);
    } else {
        PgQueryParseResult result = pg_query_parse(sql);
        if (result.error != NULL) {
            const char *message = result.error->message != NULL ? result.error->message : "unknown parser error";
            fprintf(stderr, "parser helper: SQL parse failed: %s\n", message);
            status = 1;
        } else {
            status = write_ast(result.parse_tree);
        }
        pg_query_free_parse_result(result);
    }
    free(sql);
    pg_query_exit();
    return status;
}
