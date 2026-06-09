data prices;
    input item $ q1 q2 q3 q4;
    array q{4} q1 q2 q3 q4;
    total = 0;
    do i = 1 to 4;
        total = total + q{i};
    end;
    drop i;
    datalines;
apple 10 12 14 16
bread 5 6 7 8
;
run;
