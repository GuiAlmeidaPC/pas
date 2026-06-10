/** The welcome program shown in the first editor tab of a fresh session. */
export const STARTER_PROGRAM = `/* ==================================================================== */
/* PAS (Practical Analytics Studio) — Welcome & Interactive Guide       */
/* ==================================================================== */
/* Press F3 or Cmd+Enter to execute the selected code or the whole file. */
/* Press F4 to cancel execution. Logs and datasets populate below.      */

/* STEP 1: Assigning Libraries (LIBNAME) */
/* The WORK library is always present as a temporary in-memory database. */
/* You can map folders or database files to custom libraries like this:   */
/*                                                                        */
/*   libname mydb  duckdb "path/to/my_database.duckdb";                   */
/*   libname files dir    "path/to/csv_and_parquet_folder" format=csv;    */
/*                                                                        */
/* Once mapped, you can refer to tables as 'mydb.tablename' or            */
/* read/write CSV/Parquet files directly as datasets!                     */

/* STEP 2: Basic PROC SQL (Data Generation) */
proc sql;
    create table raw_employees as
        select 'Jane Doe' as name, 'Sales' as dept, 4500 as salary union all
        select 'John Smith',       'IT',    6200 union all
        select 'Grace Hopper',     'IT',    8500 union all
        select 'Alan Turing',      'Sales', 5200;
quit;

/* STEP 3: Basic DATA Step (Filtering and Derived Columns) */
data high_earners;
    set raw_employees;
    /* Basic arithmetic and string concatenation */
    bonus = salary * 0.10;
    total_comp = salary + bonus;
    
    /* Conditional logic */
    if total_comp > 6000 then status = "High Comp";
    else status = "Standard";
run;

/* STEP 4: Advanced DATA Step (Accumulators, BY-Group Processing, FIRST/LAST) */
proc sort data=high_earners out=sorted_employees;
    by dept descending total_comp;
run;

data dept_summaries;
    set sorted_employees;
    by dept;
    
    /* Keep a running total for each department using RETAIN */
    retain dept_total_comp 0;
    if first.dept then dept_total_comp = 0;
    
    dept_total_comp = dept_total_comp + total_comp;
    
    /* Only output the final consolidated row per department */
    if last.dept;
run;

/* STEP 5: Macro Variables, Functions, & Definitions (Advanced Metaprogramming) */
%macro evaluate_bonuses(title_text, multiplier=0.15);
    %put NOTE: --- Executing macro %upcase(evaluate_bonuses) ---;
    %put NOTE: Title: &title_text;
    %put NOTE: Multiplier parameter value is: &multiplier;
    
    data macro_results;
        set raw_employees;
        /* Using macro parameters inside program statements */
        new_bonus = salary * &multiplier;
        label = "%upcase(&title_text) RESULTS";
    run;
%mend evaluate_bonuses;

/* Invoke the macro with custom positional and keyword parameters */
%evaluate_bonuses(Q2 Compensation Evaluation, multiplier=0.18);

/* STEP 6: Dynamic Macro Binding with CALL SYMPUTX */
data _null_;
    set raw_employees;
    if name = 'Grace Hopper' then do;
        /* Dynamically write to a macro variable at runtime */
        call symputx('top_employee', name);
    end;
run;

/* Print the dynamically bound value to the log pane */
%put NOTE: Top employee resolved dynamically via SYMPUTX: "&top_employee";
`;
