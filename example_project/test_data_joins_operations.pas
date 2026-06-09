/* ================================================================ */
/* Test Data + Joins + Useful Operations                            */
/* ================================================================ */

/* ----------------------------- */
/* 1. Generate test data          */
/* ----------------------------- */

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

data departments;
    length dept_id 8 dept_name $30 region $20;
    input dept_id dept_name $char30. region $char20.;
    datalines;
10 Sales                         East
20 Technology                    West
30 Analytics                     Central
40 Operations                    South
50 Finance                       East
;
run;

data sales;
    length sale_id 8 emp_id 8 sale_date 8 product $20 sale_amount 8;
    format sale_date date9. sale_amount dollar12.2;

    input sale_id emp_id sale_date :date9. product $ sale_amount;
    datalines;
1 101 05JAN2024 Software 12000
2 101 18JAN2024 Hardware 7500
3 102 20JAN2024 Software 15500
4 103 02FEB2024 Services 22000
5 103 14FEB2024 Software 18000
6 104 28FEB2024 Hardware 9200
7 105 04MAR2024 Services 17500
8 105 21MAR2024 Software 20500
9 107 29MAR2024 Hardware 11300
10 108 02APR2024 Software 19800
11 108 15APR2024 Services 16750
12 999 20APR2024 Software 5000
;
run;


/* ----------------------------- */
/* 2. Join employees to departments */
/* ----------------------------- */

proc sql;
    create table employee_department as
    select
        e.emp_id,
        e.name,
        e.dept_id,
        d.dept_name,
        d.region,
        e.hire_date,
        e.base_salary,
        intck('year', e.hire_date, today()) as years_employed
    from employees as e
    left join departments as d
        on e.dept_id = d.dept_id
    order by d.region, d.dept_name, e.name;
quit;


/* ----------------------------- */
/* 3. Aggregate sales by employee */
/* ----------------------------- */

proc sql;
    create table employee_sales_summary as
    select
        emp_id,
        count(*) as sale_count,
        sum(sale_amount) as total_sales format=dollar12.2,
        mean(sale_amount) as avg_sale_amount format=dollar12.2,
        max(sale_amount) as largest_sale format=dollar12.2
    from sales
    group by emp_id;
quit;


/* ----------------------------- */
/* 4. Join employee details to sales summaries */
/* ----------------------------- */

proc sql;
    create table employee_performance as
    select
        e.emp_id,
        e.name,
        e.dept_name,
        e.region,
        e.hire_date,
        e.years_employed,
        e.base_salary,
        coalesce(s.sale_count, 0) as sale_count,
        coalesce(s.total_sales, 0) as total_sales format=dollar12.2,
        coalesce(s.avg_sale_amount, 0) as avg_sale_amount format=dollar12.2,
        coalesce(s.largest_sale, 0) as largest_sale format=dollar12.2,

        calculated total_sales / e.base_salary as sales_to_salary_ratio format=8.2,

        case
            when calculated total_sales >= 40000 then 'Top Performer'
            when calculated total_sales >= 20000 then 'Solid Performer'
            when calculated total_sales > 0 then 'Developing'
            else 'No Sales'
        end as performance_band length=20
    from employee_department as e
    left join employee_sales_summary as s
        on e.emp_id = s.emp_id
    order by region, dept_name, calculated total_sales desc;
quit;


/* ----------------------------- */
/* 5. Find sales records with no matching employee */
/* ----------------------------- */

proc sql;
    create table unmatched_sales as
    select
        s.*
    from sales as s
    left join employees as e
        on s.emp_id = e.emp_id
    where e.emp_id is null;
quit;


/* ----------------------------- */
/* 6. Department-level summary */
/* ----------------------------- */

proc sql;
    create table department_performance as
    select
        d.dept_id,
        d.dept_name,
        d.region,
        count(distinct e.emp_id) as employee_count,
        coalesce(sum(s.sale_amount), 0) as department_sales format=dollar12.2,
        mean(e.base_salary) as avg_base_salary format=dollar12.2,
        calculated department_sales / calculated employee_count as sales_per_employee format=dollar12.2
    from departments as d
    left join employees as e
        on d.dept_id = e.dept_id
    left join sales as s
        on e.emp_id = s.emp_id
    group by
        d.dept_id,
        d.dept_name,
        d.region
    order by department_sales desc;
quit;


/* ----------------------------- */
/* 7. Rank employees within department using DATA step BY processing */
/* ----------------------------- */

proc sort data=employee_performance out=performance_sorted;
    by dept_name descending total_sales;
run;

data ranked_employee_performance;
    set performance_sorted;
    by dept_name;

    retain dept_rank;
    if first.dept_name then dept_rank = 0;

    dept_rank + 1;

    length top_in_department $3;
    if dept_rank = 1 then top_in_department = 'Yes';
    else top_in_department = 'No';
run;


/* ----------------------------- */
/* 8. Print final outputs         */
/* ----------------------------- */

proc print data=employee_performance;
    title "Employee Performance with Department and Sales Joins";
run;

proc print data=department_performance;
    title "Department-Level Performance Summary";
run;

proc print data=ranked_employee_performance;
    title "Employees Ranked Within Department by Total Sales";
run;

proc print data=unmatched_sales;
    title "Sales Records Without a Matching Employee";
run;

title;