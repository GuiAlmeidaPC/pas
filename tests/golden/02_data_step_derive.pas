proc sql;
    create table birthyears as
        select 'Ada'   as name, 1815 as born union all
        select 'Alan',          1912 union all
        select 'Grace',         1906;
quit;

data ages;
    set birthyears;
    age = 2026 - born;
    bucket = upcase(substr(name, 1, 1));
    if age < 150;
run;
