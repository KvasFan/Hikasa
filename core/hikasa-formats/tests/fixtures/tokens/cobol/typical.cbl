000100 IDENTIFICATION DIVISION.                                         PAYROLL1
000200 PROGRAM-ID. PAYROLL.                                             PAYROLL1
000300* A COMMENT LINE IN COLUMN 7                                      PAYROLL1
000400 DATA DIVISION.                                                   PAYROLL1
000500 WORKING-STORAGE SECTION.                                         PAYROLL1
000600 01  WS-TOTAL     PIC 9(7)V99 VALUE ZERO.                         PAYROLL1
000700 PROCEDURE DIVISION.                                              PAYROLL1
000800     MOVE 100 TO WS-TOTAL.                                        PAYROLL1
000900     DISPLAY "TOTAL: " WS-TOTAL.                                  PAYROLL1
001000     STOP RUN.                                                    PAYROLL1
