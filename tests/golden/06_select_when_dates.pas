data tagged;
    input id raw_date $;
    d = input(raw_date, 'date9.');
    yr = year(d);
    select;
        when (yr < 2000) era = 'pre-2000';
        when (yr <= 2020) era = '2000s-2010s';
        otherwise            era = '2020s+';
    end;
    formatted = put(d, 'yymmdd10.');
    datalines;
1 01JAN1995
2 15JUN2010
3 31DEC2024
;
run;
