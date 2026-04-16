# nitrocop-config: EnforcedStyle: space
select( "DISTINCT ON(LOWER(miq_reports.name), miq_report_results.miq_report_id) LOWER(miq_reports.name), \
       miq_report_results.miq_report_id")
