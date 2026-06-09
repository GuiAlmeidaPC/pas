/* Exercise do while and do until in a DATA step. */
proc sql;
    create table seed as
        select 1 as id, 0 as start_val union all
        select 2,        5             union all
        select 3,        10;
quit;

data accumulated;
    set seed;
    /* do while: condition tested at top; may execute zero times */
    sum_while = start_val;
    do while (sum_while < 8);
        sum_while = sum_while + 1;
    end;

    /* do until: condition tested at bottom; body runs at least once */
    sum_until = start_val;
    do until (sum_until >= 7);
        sum_until = sum_until + 2;
    end;
run;
