proc sql;
    create table customers as select * from (values
        (1, 'Ada'),
        (2, 'Alan'),
        (3, 'Grace')
    ) as t(id, name);
    create table orders as select * from (values
        (1, 100),
        (1, 50),
        (2, 75)
    ) as t(id, amount);
quit;

data joined;
    merge customers orders;
    by id;
run;
