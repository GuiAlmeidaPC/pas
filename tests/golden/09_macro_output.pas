%macro generate_data(lib, dataset, count=3, filter_val=active);
    %put NOTE: Starting macro execution...;
    %put NOTE: Library: &lib, Dataset: &dataset;
    %put NOTE: Count parameter is &count;

    /* Upcase the filter value using built-in %upcase function */
    %let clean_filter = %upcase(&filter_val);
    %put NOTE: Filter value upper-cased to: &clean_filter;

    /* Generate a standard PAS DATA step */
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

/* Invoke the Macro */
%generate_data(work, sample_items, count=5, filter_val=active);
