proc sql;
    create table source as
    select x from range(1, 6) t(x);
quit;

data improved;
    set source(
        firstobs=2
        obs=4
        where=(x ne 3)
        keep=x
        rename=(x=value)
        in=from_source
    );
    iteration = _n_;
    if value = 2 then return;
    after_return = 1;
    format value 8.;
    informat value 8.;
    label value="Selected value";
run;
