proc sql;
    create table letters as
        select 'a' as letter, 1 as ord union all
        select 'b', 2 union all
        select 'c', 3;
    select * from letters order by ord;
quit;
