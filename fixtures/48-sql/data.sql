-- Built into a SQLite database by the fixture tests and tools/compare.py.
CREATE TABLE dept (id INTEGER PRIMARY KEY, name TEXT NOT NULL, city TEXT);
CREATE TABLE emp (id INTEGER PRIMARY KEY, dept_id INTEGER NOT NULL, name TEXT NOT NULL, salary REAL, hired TEXT, active INTEGER);
INSERT INTO dept VALUES (1, 'Engineering', 'Santo Domingo'), (2, 'Finance', 'Santiago'), (3, 'Sales', 'Punta Cana'), (4, 'Support', NULL);
INSERT INTO emp VALUES
 (1, 1, 'Ana Pérez', 98000.5, '2021-03-15', 1),
 (2, 1, 'Luis Gómez', 87500, '2019-11-02', 1),
 (3, 1, 'María Núñez', 120000, '2016-06-30', 1),
 (4, 1, 'Pedro Rosario', 45000, '2024-01-08', 0),
 (5, 2, 'Carmen Díaz', 76000.25, '2018-09-12', 1),
 (6, 2, 'José Marte', 52000, '2022-04-19', 1),
 (7, 3, 'Rosa Taveras', 61000, '2020-02-01', 1),
 (8, 3, 'Juan Batista', 39000, '2023-07-23', 1),
 (9, 3, 'Elena Castillo', 72500.75, '2017-12-05', 1),
 (10, 4, 'Miguel Santos', 41000, '2021-08-17', 1);
