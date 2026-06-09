proc sql;
    create table sales as select * from (values
        ('east','q1', 10),
        ('east','q2', 20),
        ('east','q1', 5),
        ('west','q1', 30),
        ('west','q2', 25)
    ) as t(region, qtr, amount);
quit;

proc sort data=sales out=sorted;
    by region descending amount;
run;

proc transpose data=sorted out=wide;
    by region;
    id qtr;
    var amount;
run;
