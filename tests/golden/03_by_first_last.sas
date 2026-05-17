proc sql;
    create table sales as select * from (values
        ('east','phone',10),
        ('east','phone',15),
        ('east','book',5),
        ('west','phone',20),
        ('west','book',8),
        ('west','book',2)
    ) as t(region, item, qty);
quit;

data totals;
    set sales;
    by region;
    retain total 0;
    if first.region then total = 0;
    total = total + qty;
    if last.region;
run;
