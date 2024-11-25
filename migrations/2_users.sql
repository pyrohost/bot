create table if not exists users 
(
    id                      integer primary key,
    modrinth_id             text,
    -- store the testing servers as JSON :) 
    testing_servers         text,
    max_testing_servers     integer
)
