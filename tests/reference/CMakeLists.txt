cmake_minimum_required(VERSION 3.15)
project(dspadpcm_ref C)

add_executable(dspadpcm_ref main.c grok.c)
target_compile_options(dspadpcm_ref PRIVATE
    $<$<C_COMPILER_ID:MSVC>:/W0 /std:c11>
    $<$<NOT:$<C_COMPILER_ID:MSVC>>:-O2 -Wall>
)
target_link_libraries(dspadpcm_ref PRIVATE
    $<$<NOT:$<C_COMPILER_ID:MSVC>>:m>
)
