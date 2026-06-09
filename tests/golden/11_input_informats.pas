/* Informat / column input: modified-list (:date9.), formatted ($char40.),
   and the format statement are all accepted and read. */
data employees;
    length emp_id 8 name $40 dept_id 8 hire_date 8 base_salary 8;
    format hire_date date9. base_salary dollar12.2;

    input emp_id name $char40. dept_id hire_date :date9. base_salary;
    datalines;
101 Jane Doe                              10 15JAN2021 62000
102 John Smith                            20 03MAR2020 78000
103 Grace Hopper                          20 22JUL2019 95000
104 Alan Turing                           10 11NOV2022 71000
105 Katherine Johnson                     30 08FEB2021 88000
106 Ada Lovelace                          40 19SEP2023 67000
107 Mary Jackson                          30 27APR2020 74000
108 Tim Berners-Lee                       20 14JUN2022 83000
;
run;
