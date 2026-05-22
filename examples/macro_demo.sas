/* ==================================================================== */
/* PAS Macro Engine & CALL SYMPUT Demo                                  */
/* ==================================================================== */

/* 1. Basic Macro Variable Assignment and Print */
%let greeting = Hello from the PAS Macro Preprocessor;
%put NOTE: Greeting is: &greeting;

/* 2. Macro Definition with Positional and Keyword Parameters */
%macro generate_data(lib, dataset, count=3, filter_val=active);
    %put NOTE: Starting macro execution...;
    %put NOTE: Library: &lib, Dataset: &dataset;
    %put NOTE: Count parameter is &count;

    /* Upcase the filter value using built-in %upcase function */
    %let clean_filter = %upcase(&filter_val);
    %put NOTE: Filter value upper-cased to: &clean_filter;

    /* Generate a standard SAS DATA step */
    data &lib..&dataset;
        length name $15 status $15 category $15;
        
        name = "Item_1";
        status = "&clean_filter";
        category = "A";
        output;
        
        name = "Item_2";
        status = "INACTIVE";
        category = "B";
        output;
        
        name = "Item_3";
        status = "&clean_filter";
        category = "A";
        output;
    run;
%mend generate_data;

/* 3. Invoke the Macro */
%generate_data(work, sample_items, count=5, filter_val=active);

/* 4. Query the generated dataset using PROC SQL */
proc sql;
    select * from work.sample_items where status = 'ACTIVE';
quit;

/* 5. Dynamic Variable Binding via CALL SYMPUT & CALL SYMPUTX */
data _null_;
    set work.sample_items;
    if name = 'Item_3' then do;
        /* CALL SYMPUT preserves leading/trailing spaces */
        call symput('saved_item', name);
        /* CALL SYMPUTX automatically trims leading/trailing spaces */
        call symputx('trimmed_status', status);
    end;
run;

/* 6. Print the dynamically bound macro variables in subsequent block */
%put NOTE: Saved Item Name via SYMPUT:  "&saved_item";
%put NOTE: Trimmed Status via SYMPUTX: "&trimmed_status";
