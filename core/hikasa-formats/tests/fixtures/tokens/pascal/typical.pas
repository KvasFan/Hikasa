unit Orders;

{$mode objfpc}{$H+}

interface

uses
  SysUtils, Classes;

type
  TOrder = class(TObject)
  private
    FTotal: Currency;
  public
    procedure Add(const Amount: Currency);
    property Total: Currency read FTotal;
  end;

implementation

{ TOrder }

procedure TOrder.Add(const Amount: Currency);
begin
  // ignore negative amounts
  if Amount > 0 then
    FTotal := FTotal + Amount;
end;

end.
