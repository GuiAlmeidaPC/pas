/* Column-range input reads fixed-width fields, including names with
   embedded spaces, without the list/formatted pointer pitfalls. */
data employees;
    length emp_id 8 name $20 dept_id 8;
    input emp_id 1-3 name $ 5-24 dept_id 26-27;
    datalines;
101 Jane Doe             10
102 John Smith           20
103 Grace Hopper         20
104 Alan Turing          10
;
run;
