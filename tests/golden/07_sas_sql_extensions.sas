proc sql;
    create table people as select * from (values
        (1, 'Ada'),
        (2, 'Alan'),
        (3, 'Grace')
    ) as t(id, name);

    create table scores as select * from (values
        ('Ada', 95),
        ('Grace', 88)
    ) as t(name, score);

    create table indexed as
        select monotonic() as rn, id, name, name || ' #' || cast(id as varchar) as label,
               calculated label as alias
        from people;

    create table unioned as
        select id, name from people
        outer union corr
        select name, score from scores;
quit;
