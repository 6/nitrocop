# nitrocop-config: EnforcedStyle: compact
select( "DISTINCT ON(LOWER(miq_reports.name), miq_report_results.miq_report_id) LOWER(miq_reports.name), \
       miq_report_results.miq_report_id")
f( x( 3 ))
