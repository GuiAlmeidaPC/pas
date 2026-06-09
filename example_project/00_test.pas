proc sql;
    create table raw_data_00 as
    select 'Apple' as item, 10 as qty, 0.5 as price union all
    select 'Banana', 20, 0.2 union all
    select 'Cherry', 15, 1.0;
quit;
