create table if not exists guilds 
(
    id                      integer primary key,
    stats_category          integer, 
    nodes_channel           integer,
    network_channel         integer,
    network_total_channel   integer,
    storage_channel         integer,
    memory_channel          integer,
    lorax_role              integer,
    lorax_channel           integer,
    -- store the lorax state as JSON
    lorax_state             text 
)
